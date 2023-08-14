use std::collections;
use std::ops::{Add, Deref};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use hyper::{Body, Request, Response, StatusCode};
use log::{error, info, warn};
use regex::Regex;
use serde_json;
use tokio;

use octobot_lib::config::Config;
use octobot_lib::errors::Result;
use octobot_lib::github;
use octobot_lib::github::api::Session;
use octobot_lib::github::CommentLike;
use octobot_lib::jira;
use octobot_lib::metrics::{self, Metrics};
use octobot_ops::force_push::{self, ForcePushRequest};
use octobot_ops::git_clone_manager::GitCloneManager;
use octobot_ops::messenger::{self, Messenger};
use octobot_ops::pr_merge::{self, PRMergeRequest};
use octobot_ops::repo_version::{self, RepoVersionRequest};
use octobot_ops::slack::{self, Slack, SlackAttachmentBuilder, SlackRequest};
use octobot_ops::util;
use octobot_ops::webhook_db::WebhookDatabase;
use octobot_ops::worker::{TokioWorker, Worker};

use crate::http_util;
use crate::runtime;
use crate::server::github_verify::GithubWebhookVerifier;
use crate::server::http::Handler;

pub struct GithubHandlerState {
    pub config: Arc<Config>,
    pub github_app: Arc<dyn github::api::GithubSessionFactory>,
    pub jira_session: Option<Arc<dyn jira::api::Session>>,
    _runtime: Arc<Mutex<tokio::runtime::Runtime>>,
    pr_merge_worker: Arc<dyn Worker<PRMergeRequest>>,
    repo_version_worker: Arc<dyn Worker<RepoVersionRequest>>,
    force_push_worker: Arc<dyn Worker<ForcePushRequest>>,
    slack_worker: Arc<dyn Worker<SlackRequest>>,
    webhook_db: Arc<WebhookDatabase>,
    metrics: Arc<Metrics>,
    git_clone_manager: Arc<GitCloneManager>,
}

pub struct GithubHandler {
    state: Arc<GithubHandlerState>,
}

pub struct GithubEventHandler {
    pub messenger: Messenger,
    pub config: Arc<Config>,
    pub event: String,
    pub data: github::HookBody,
    pub repository: github::Repo,
    pub action: String,
    pub github_session: Arc<dyn github::api::Session>,
    pub jira_session: Option<Arc<dyn jira::api::Session>>,
    pub pr_merge: Arc<dyn Worker<PRMergeRequest>>,
    pub repo_version: Arc<dyn Worker<RepoVersionRequest>>,
    pub force_push: Arc<dyn Worker<ForcePushRequest>>,
    pub team_members_cache: TeamsCache,
}

struct TeamCacheEntry {
    users: Vec<github::User>,
    expiry: Instant,
}

pub struct TeamsCache {
    members: Mutex<collections::HashMap<(u32, u32), TeamCacheEntry>>,
    ttl: Duration,
}

impl TeamsCache {
    pub fn new(ttl: Duration) -> TeamsCache {
        TeamsCache {
            members: Mutex::new(collections::HashMap::new()),
            ttl,
        }
    }

    pub fn insert(&self, repo: &github::Repo, team_id: u32, users: Vec<github::User>) {
        let mut hash = self.members.lock().unwrap();
        let entry = TeamCacheEntry {
            users,
            expiry: Instant::now().add(self.ttl),
        };
        hash.insert((repo.owner.id, team_id), entry);
    }

    pub fn get(&self, repo: &github::Repo, team_id: u32) -> Option<Vec<github::User>> {
        let key = (repo.owner.id, team_id);
        let mut hash = self.members.lock().unwrap();
        let entry = hash.get(&key)?;
        if Instant::now() > entry.expiry {
            hash.remove(&key);
            return None;
        }
        Some(entry.users.clone())
    }
}

const MAX_CONCURRENT_JOBS: usize = 20;
const MAX_COMMITS_FOR_JIRA_CONSIDERATION: usize = 20;

impl GithubHandlerState {
    pub fn new(
        config: Arc<Config>,
        github_app: Arc<dyn github::api::GithubSessionFactory>,
        jira_session: Option<Arc<dyn jira::api::Session>>,
        slack: Arc<Slack>,
        webhook_db: Arc<WebhookDatabase>,
        metrics: Arc<Metrics>,
    ) -> GithubHandlerState {
        let git_clone_manager = Arc::new(GitCloneManager::new(github_app.clone(), config.clone()));

        let runtime = Arc::new(Mutex::new(runtime::new(
            MAX_CONCURRENT_JOBS,
            "jobs",
            metrics.clone(),
        )));

        let slack_worker =
            TokioWorker::new_worker(runtime.clone(), slack::new_runner(slack.clone()));
        let pr_merge_worker = TokioWorker::new_worker(
            runtime.clone(),
            pr_merge::new_runner(
                config.clone(),
                github_app.clone(),
                git_clone_manager.clone(),
                slack_worker.clone(),
                metrics.clone(),
            ),
        );
        let repo_version_worker = TokioWorker::new_worker(
            runtime.clone(),
            repo_version::new_runner(
                config.clone(),
                github_app.clone(),
                jira_session.clone(),
                git_clone_manager.clone(),
                slack_worker.clone(),
                metrics.clone(),
            ),
        );
        let force_push_worker = TokioWorker::new_worker(
            runtime.clone(),
            force_push::new_runner(
                github_app.clone(),
                git_clone_manager.clone(),
                metrics.clone(),
            ),
        );

        GithubHandlerState {
            config,
            github_app: github_app.clone(),
            jira_session: jira_session.clone(),
            _runtime: runtime,
            pr_merge_worker,
            repo_version_worker,
            force_push_worker,
            slack_worker,
            webhook_db,
            metrics,
            git_clone_manager,
        }
    }

    pub fn clean(&self) {
        let hour = Duration::from_secs(3600);
        let day = 24 * hour;

        self.git_clone_manager.clean(1 * day);

        if let Err(e) = self.webhook_db.clean(SystemTime::now() - (7 * day)) {
            log::error!("Failed to clean webhook db: {}", e);
        }
    }
}

impl GithubHandler {
    pub fn new(
        config: Arc<Config>,
        github_app: Arc<dyn github::api::GithubSessionFactory>,
        jira_session: Option<Arc<dyn jira::api::Session>>,
        slack: Arc<Slack>,
        webhook_db: Arc<WebhookDatabase>,
        metrics: Arc<Metrics>,
    ) -> Box<GithubHandler> {
        let state = GithubHandlerState::new(config, github_app, jira_session, slack, webhook_db, metrics);
        GithubHandler::from_state(Arc::new(state))
    }

    pub fn from_state(state: Arc<GithubHandlerState>) -> Box<GithubHandler> {
        Box::new(GithubHandler { state })
    }
}

#[async_trait::async_trait]
impl Handler for GithubHandler {
    async fn handle(&self, req: Request<Body>) -> Result<Response<Body>> {
        Ok(self.handle_ok(req).await)
    }

    async fn handle_ok(&self, req: Request<Body>) -> Response<Body> {
        let _scoped_count = metrics::scoped_inc(&self.state.metrics.current_webhook_count);

        let event_id;
        {
            let values = req
                .headers()
                .get_all("x-github-delivery")
                .iter()
                .collect::<Vec<_>>();
            if values.len() != 1 {
                let msg = "Expected to find exactly one event id header";
                error!("{}", msg);
                return http_util::new_bad_req_resp(msg);
            }

            event_id = String::from_utf8_lossy(values[0].as_bytes()).into_owned();
        }

        // make sure event id is valid
        match self.state.webhook_db.maybe_record(&event_id) {
            Err(e) => {
                error!("Failed to record webhook event guid {}: {}", event_id, e);
            }
            Ok(true) => {
                log::trace!("Recorded new webhook event: {}", event_id);
            }
            Ok(false) => {
                let msg = format!("Duplicate webhook event: {}", event_id);
                error!("{}", msg);
                return http_util::new_bad_req_resp(msg);
            }
        };

        let event;
        {
            let values = req
                .headers()
                .get_all("x-github-event")
                .iter()
                .collect::<Vec<_>>();
            if values.len() != 1 {
                let msg = "Expected to find exactly one event header";
                error!("{}", msg);
                return http_util::new_bad_req_resp(msg);
            }
            event = String::from_utf8_lossy(values[0].as_bytes()).into_owned();
        }

        let headers = req.headers().clone();
        let github_app = self.state.github_app.clone();
        let config = self.state.config.clone();
        let jira_session = self.state.jira_session.clone();
        let pr_merge = self.state.pr_merge_worker.clone();
        let repo_version = self.state.repo_version_worker.clone();
        let force_push = self.state.force_push_worker.clone();
        let slack = self.state.slack_worker.clone();

        let body = match hyper::body::to_bytes(req.into_body()).await {
            Ok(b) => b,
            Err(e) => {
                error!("Error reading request body: {}", e);
                return http_util::new_bad_req_resp(format!("Error reading request body: {}", e));
            }
        };

        let verifier = GithubWebhookVerifier {
            secret: config.github.webhook_secret.clone(),
        };
        if !verifier.is_req_valid(&headers, &body) {
            return http_util::new_msg_resp(StatusCode::FORBIDDEN, "Invalid signature");
        }

        let mut data: github::HookBody = match serde_json::from_slice(&body) {
            Ok(h) => h,
            Err(e) => {
                error!(
                    "Error parsing json: {}\n---\n{}\n---\n",
                    e,
                    String::from_utf8_lossy(&body)
                );
                return http_util::new_bad_req_resp(format!("Error parsing JSON: {}", e));
            }
        };

        // Installation events have no repository, ignore
        let repository = match data.repository {
            Some(ref r) => r.clone(),
            None => return http_util::new_msg_resp(StatusCode::OK, "no repository, ignored"),
        };

        let github_session = match github_app
            .new_session(repository.owner.login(), &repository.name)
            .await
        {
            // Note: this doesn't really need to be an Arc anymore...
            Ok(g) => Arc::new(g),
            Err(e) => {
                error!(
                    "Error creating a new github session for {}/{}: {}",
                    repository.owner.login(),
                    &repository.name,
                    e
                );
                return http_util::new_bad_req_resp("Could not create github session");
            }
        };

        let action = match data.action {
            Some(ref a) => a.clone(),
            None => String::new(),
        };

        // Try to remap issues which are PRs as pull requests. This gives us access to PR information
        // like reviewers which do not exist for issues.
        if let Some(ref issue) = data.issue {
            if data.pull_request.is_none() && issue.html_url.contains("/pull/") {
                data.pull_request = match github_session
                    .get_pull_request(repository.owner.login(), &repository.name, issue.number)
                    .await
                {
                    Ok(pr) => Some(pr),
                    Err(e) => {
                        error!(
                            "Error refetching issue #{} as pull request: {}",
                            issue.number, e
                        );
                        None
                    }
                };
            }
        }

        // refetch PR if present to get requested reviewers: they don't come on each webhook :cry:
        let mut changed_pr = None;
        if let Some(ref pull_request) = data.pull_request {
            if pull_request.requested_reviewers.is_none() || pull_request.reviews.is_none() {
                match github_session
                    .get_pull_request(
                        repository.owner.login(),
                        &repository.name,
                        pull_request.number,
                    )
                    .await
                {
                    Ok(mut refetched_pr) => {
                        if refetched_pr.draft != pull_request.draft {
                            warn!(
                                "Refetched pull request had mismatched draft: {:?} != {:?}",
                                refetched_pr.draft, pull_request.draft
                            );
                            refetched_pr.draft = pull_request.draft;
                        }
                        if refetched_pr.head.sha != pull_request.head.sha {
                            warn!(
                                "Refetched pull request had different HEAD commit hash: {:?} != {:?}",
                                refetched_pr.head.sha, pull_request.head.sha
                            );
                        }

                        changed_pr = Some(refetched_pr);
                    }
                    Err(e) => error!("Error refetching pull request to get reviewers: {}", e),
                };
            }
        }
        if let Some(changed_pr) = changed_pr {
            data.pull_request = Some(changed_pr);
        }

        let handler = GithubEventHandler {
            event: event.clone(),
            data,
            repository,
            action,
            config: config.clone(),
            messenger: messenger::new(config.clone(), slack),
            github_session,
            jira_session,
            pr_merge,
            repo_version,
            force_push,
            team_members_cache: TeamsCache::new(Duration::from_secs(3600)),
        };

        match handler.handle_event().await {
            Some((status, resp)) => http_util::new_msg_resp(status, resp),
            None => http_util::new_msg_resp(StatusCode::OK, format!("Unhandled event: {}", event)),
        }
    }
}

type EventResponse = (StatusCode, String);

impl GithubEventHandler {
    pub async fn handle_event(&self) -> Option<EventResponse> {
        info!(
            "Received event: {}{}{}",
            self.event,
            if self.action.is_empty() { "" } else { "." },
            self.action
        );
        if self.event == "ping" {
            Some(self.handle_ping())
        } else if self.event == "pull_request" {
            Some(self.handle_pr().await)
        } else if self.event == "pull_request_review_comment" {
            Some(self.handle_pr_review_comment().await)
        } else if self.event == "pull_request_review" {
            Some(self.handle_pr_review().await)
        } else if self.event == "commit_comment" {
            Some(self.handle_commit_comment().await)
        } else if self.event == "issue_comment" {
            Some(self.handle_issue_comment().await)
        } else if self.event == "push" {
            Some(self.handle_push().await)
        } else {
            None
        }
    }

    // This defaults to using the github name if no slack name is configured, since this is not
    // used for actually sending messages, but just for referring to users in slack messages.
    fn slack_user_name(&self, user: &github::User) -> String {
        match self.config.users().slack_user_name(user.login()) {
            Some(slack_user) => slack_user,
            None => user.login().to_string(),
        }
    }

    fn slack_user_names(&self, users: &[github::User]) -> Vec<String> {
        users.iter().map(|u| self.slack_user_name(u)).collect()
    }

    async fn pull_request_commits(
        &self,
        pull_request: &dyn github::PullRequestLike,
    ) -> Vec<github::Commit> {
        if !pull_request.has_commits() {
            return vec![];
        }

        match self
            .github_session
            .get_pull_request_commits(
                self.repository.owner.login(),
                &self.repository.name,
                pull_request.number(),
            )
            .await
        {
            Ok(commits) => commits,
            Err(e) => {
                error!("Error looking up PR commits: {}", e);
                vec![]
            }
        }
    }

    async fn all_participants(
        &self,
        pull_request: &dyn github::PullRequestLike,
        pr_commits: &[github::Commit],
    ) -> Vec<github::User> {
        // start with the assignees
        let mut participants = pull_request.assignees();
        // add the author of the PR
        participants.push(pull_request.user().clone());
        // add team participants
        participants.extend(self.team_participants(pull_request).await);

        // look up commits and add the authors of those
        for commit in pr_commits {
            if let Some(ref author) = commit.author {
                participants.push(author.clone());
            }
        }

        participants.sort_by(|a, b| a.login().cmp(b.login()));
        participants.dedup();
        participants
    }

    async fn team_participants(
        &self,
        pull_request: &dyn github::PullRequestLike,
    ) -> Vec<github::User> {
        let mut participants = vec![];

        // Issues do not have a repo and cannot have teams assigned
        let repo = match pull_request.repo() {
            None => {
                return participants;
            }
            Some(r) => r,
        };

        // add team members
        let teams = pull_request.teams();

        for t in teams {
            let team_members = self.team_members_cache.get(repo, t.id);
            if let Some(team_members) = team_members {
                participants.extend(team_members.into_iter());
            } else {
                let team_members = self.github_session.get_team_members(repo, t.id).await;
                match team_members {
                    Ok(m) => {
                        participants.extend(m.clone().into_iter());
                        self.team_members_cache.insert(repo, t.id, m);
                    }
                    Err(e) => {
                        error!("Error getting team members: {}", e);
                    }
                }
            }
        }

        participants
    }

    fn reviewer_names(&self, pull_request: &github::PullRequest) -> Vec<String> {
        let mut reviewer_names = vec![];

        if let Some(ref reviewers) = pull_request.requested_reviewers {
            reviewer_names.extend(self.slack_user_names(reviewers));
        }
        if let Some(ref teams) = pull_request.requested_teams {
            reviewer_names.extend(teams.into_iter().map(|t| format!("@{}", t.slug)));
        }

        reviewer_names
    }

    fn handle_ping(&self) -> EventResponse {
        (StatusCode::OK, "ping".into())
    }

    async fn handle_pr(&self) -> EventResponse {
        enum NotifyMode {
            All,
            Channel,
            None,
        }

        if let Some(ref pull_request) = self.data.pull_request {
            let verb: Option<String>;
            let notify_mode;
            if self.action == "opened" {
                verb = Some(format!(
                    "opened by {}",
                    self.slack_user_name(&pull_request.user)
                ));
                notify_mode = NotifyMode::Channel;
            } else if self.action == "closed" {
                if pull_request.merged == Some(true) {
                    verb = Some("merged".to_string());
                } else {
                    verb = Some("closed".to_string());
                }
                notify_mode = NotifyMode::All;
            } else if self.action == "reopened" {
                verb = Some("reopened".to_string());
                notify_mode = NotifyMode::Channel;
            } else if self.action == "edited" {
                verb = Some("edited".to_string());
                notify_mode = NotifyMode::None;
            } else if self.action == "ready_for_review" {
                verb = Some("is ready for review".to_string());
                notify_mode = NotifyMode::All;
            } else if self.action == "assigned" {
                let assignees_str = self.slack_user_names(&pull_request.assignees).join(", ");
                verb = Some(format!("assigned to {}", assignees_str));
                notify_mode = NotifyMode::All;
            } else if self.action == "unassigned" {
                verb = Some("unassigned".to_string());
                notify_mode = NotifyMode::Channel;
            } else if self.action == "review_requested" {
                let mut reviewers_str = self.reviewer_names(pull_request).join(", ");
                if reviewers_str.is_empty() {
                    reviewers_str = "<nobody>".into();
                }
                verb = Some(format!(
                    "by {} submitted for review to {}",
                    self.slack_user_name(&pull_request.user),
                    reviewers_str
                ));

                notify_mode = NotifyMode::All;
            } else if self.action == "synchronize" {
                verb = Some("synchronize".to_string());
                notify_mode = NotifyMode::None;
            } else {
                verb = None;
                notify_mode = NotifyMode::None;
            }

            // early exit if we have nothing to do here.
            if verb.is_none() && self.action != "labeled" {
                return (StatusCode::OK, "pr".into());
            }

            let commits = self.pull_request_commits(&pull_request).await;

            if let Some(ref verb) = verb {
                let branch_name = &pull_request.base.ref_name;

                let attachments = vec![SlackAttachmentBuilder::new("")
                    .title(format!(
                        "Pull Request #{}: \"{}\"",
                        pull_request.number,
                        pull_request.title.as_str()
                    ))
                    .title_link(pull_request.html_url.as_str())
                    .build()];

                if !pull_request.is_draft() {
                    let msg = format!("Pull Request {}", verb);
                    let thread_guid = self.build_thread_guid(pull_request.number);
                    match notify_mode {
                        NotifyMode::Channel => self.messenger.send_to_channel(
                            &msg,
                            &attachments,
                            &self.repository,
                            branch_name,
                            &commits,
                            vec![thread_guid],
                            self.action == "opened",
                        ),

                        NotifyMode::All => self.messenger.send_to_all(
                            &msg,
                            &attachments,
                            &pull_request.user,
                            &self.data.sender,
                            &self.repository,
                            &self.all_participants(&pull_request, &commits).await,
                            branch_name,
                            &commits,
                            vec![thread_guid],
                        ),

                        NotifyMode::None => (),
                    };
                }

                let jira_projects = self
                    .config
                    .repos()
                    .jira_projects(&self.repository, branch_name);

                let is_pull_request_first_ready =
                    self.action == "opened" || self.action == "ready_for_review";

                // Mark JIRAs in review for PR open
                if is_pull_request_first_ready {
                    if let Some(ref jira_config) = self.config.jira {
                        if let Some(ref jira_session) = self.jira_session {
                            if commits.len() > MAX_COMMITS_FOR_JIRA_CONSIDERATION {
                                let msg = format!(
                                    "Too many commits on Pull Request #{}. Ignoring JIRAs.",
                                    pull_request.number
                                );
                                self.messenger.send_to_owner(
                                    &msg,
                                    &attachments,
                                    &pull_request.user,
                                    &self.repository,
                                    branch_name,
                                    &commits,
                                );
                            } else {
                                jira::workflow::submit_for_review(
                                    pull_request,
                                    &commits,
                                    &jira_projects,
                                    jira_session.deref(),
                                    jira_config,
                                )
                                .await;
                            }
                        }
                    }
                }

                // Check for jira reference on ready for review and PR title rename
                // (since JIRA check ignore is based on PR title)
                if is_pull_request_first_ready
                    || self.action == "edited"
                    || self.action == "synchronize"
                {
                    // Mark if no JIRA references
                    jira::check_jira_refs(
                        pull_request,
                        &commits,
                        &jira_projects,
                        self.github_session.deref(),
                    )
                    .await;
                }
            }

            let release_branch_prefix = self.config.repos().release_branch_prefix(&self.repository);
            if self.action == "labeled" {
                if let Some(ref label) = self.data.label {
                    self.merge_pull_request(pull_request, label, &release_branch_prefix, &commits);
                }
            } else if verb == Some("merged".to_string()) {
                self.merge_pull_request_all_labels(pull_request, &release_branch_prefix, &commits)
                    .await;
            }
        }

        (StatusCode::OK, "pr".into())
    }

    async fn handle_pr_review_comment(&self) -> EventResponse {
        if let Some(ref pull_request) = self.data.pull_request {
            if let Some(ref comment) = self.data.comment {
                if self.action == "created" {
                    let branch_name = &pull_request.base.ref_name;
                    let commits = self.pull_request_commits(&pull_request).await;

                    self.do_pull_request_comment(&pull_request, &comment, branch_name, &commits)
                        .await;
                }
            }
        }

        (StatusCode::OK, "pr_review_comment".into())
    }

    async fn handle_pr_review(&self) -> EventResponse {
        if let Some(ref pull_request) = self.data.pull_request {
            if let Some(ref review) = self.data.review {
                if self.action == "submitted" {
                    let branch_name = &pull_request.base.ref_name;
                    let commits = self.pull_request_commits(&pull_request).await;

                    // just a comment. should just be handled by regular comment handler.
                    if review.state == "commented" {
                        self.do_pull_request_comment(&pull_request, &review, branch_name, &commits)
                            .await;
                        return (StatusCode::OK, "pr_review [comment]".into());
                    }

                    let action_msg;
                    let state_msg;
                    let color;
                    if review.state == "changes_requested" {
                        action_msg = "requested changes to";
                        state_msg = "Changes Requested";
                        color = "danger";
                    } else if review.state == "approved" {
                        action_msg = "approved";
                        state_msg = "Approved";
                        color = "good";
                    } else {
                        return (StatusCode::OK, "pr_review [ignored]".into());
                    }

                    let msg = format!(
                        "{} {} PR \"{}\"",
                        self.slack_user_name(&review.user),
                        action_msg,
                        util::make_link(
                            pull_request.html_url.as_str(),
                            pull_request.title.as_str(),
                        )
                    );

                    let attachments = vec![SlackAttachmentBuilder::new(review.body())
                        .title(format!("Review: {}", state_msg))
                        .title_link(review.html_url.as_str())
                        .color(color)
                        .build()];

                    let mut participants = self.all_participants(&pull_request, &commits).await;
                    for username in util::get_mentioned_usernames(review.body()) {
                        participants.push(github::User::new(username))
                    }

                    self.messenger.send_to_all(
                        &msg,
                        &attachments,
                        &pull_request.user,
                        &self.data.sender,
                        &self.repository,
                        &participants,
                        branch_name,
                        &commits,
                        vec![self.build_thread_guid(pull_request.number)],
                    );
                }
            }
        }

        (StatusCode::OK, "pr_review".into())
    }

    async fn do_pull_request_comment(
        &self,
        pull_request: &dyn github::PullRequestLike,
        comment: &dyn github::CommentLike,
        branch_name: &str,
        commits: &[github::Commit],
    ) {
        if comment.body().trim().is_empty() {
            return;
        }

        if comment.user().login() == self.github_session.bot_name()
            || self
                .config
                .slack
                .ignored_users
                .contains(&comment.user().login().to_string())
        {
            info!(
                "Ignoring message from bot ({}): {}",
                comment.user().login(),
                comment.body()
            );
            return;
        }

        let msg = format!(
            "Comment on \"{}\"",
            util::make_link(pull_request.html_url(), pull_request.title())
        );

        let attachments = vec![SlackAttachmentBuilder::new(comment.body().trim())
            .title(format!("{} said:", self.slack_user_name(comment.user())))
            .title_link(comment.html_url())
            .build()];

        let mut participants = self.all_participants(pull_request, commits).await;
        for username in util::get_mentioned_usernames(comment.body()) {
            participants.push(github::User::new(username))
        }

        self.messenger.send_to_all(
            &msg,
            &attachments,
            pull_request.user(),
            &self.data.sender,
            &self.repository,
            &participants,
            branch_name,
            commits,
            vec![self.build_thread_guid(pull_request.number())],
        );
    }

    async fn handle_commit_comment(&self) -> EventResponse {
        if let Some(ref comment) = self.data.comment {
            if self.action == "created" {
                if let Some(ref commit_id) = comment.commit_id {
                    let commit: &str = &commit_id[0..7];
                    let commit_url = format!("{}/commit/{}", self.repository.html_url, commit_id);
                    let commit_path = if let Some(ref path) = comment.path {
                        path.to_string()
                    } else {
                        commit.to_string()
                    };

                    let msg = format!(
                        "Comment on \"{}\" ({})",
                        commit_path,
                        util::make_link(commit_url.as_str(), commit)
                    );

                    let attachments = vec![SlackAttachmentBuilder::new(comment.body())
                        .title(format!("{} said:", self.slack_user_name(&comment.user)))
                        .title_link(comment.html_url.as_str())
                        .build()];

                    // TODO: should try to tie this back to a PR to get this to the right channel.
                    let branch_name = "";
                    let commits = Vec::<github::Commit>::new();

                    let commit_prs = match self
                        .github_session
                        .get_pull_requests_by_commit(
                            self.repository.owner.login(),
                            &self.repository.name,
                            commit,
                            None,
                        )
                        .await
                    {
                        Ok(p) => p,
                        Err(e) => {
                            error!(
                                "Error looking up PR for '{}' ({}): {}",
                                branch_name,
                                self.data.after(),
                                e
                            );
                            return (StatusCode::OK, "push [no PR]".into());
                        }
                    };
                    let thread_guids = commit_prs
                        .into_iter()
                        .map(|pr| self.build_thread_guid(pr.number))
                        .collect();
                    self.messenger.send_to_all(
                        &msg,
                        &attachments,
                        &comment.user,
                        &self.data.sender,
                        &self.repository,
                        &[],
                        branch_name,
                        &commits,
                        thread_guids,
                    );
                }
            }
        }

        (StatusCode::OK, "commit_comment".into())
    }

    async fn handle_issue_comment(&self) -> EventResponse {
        if let Some(ref comment) = self.data.comment {
            if self.action == "created" {
                // Check to see if we remapped this "issue" to a PR
                if let Some(ref pr) = self.data.pull_request {
                    let branch_name = &pr.base.ref_name;
                    let commits = self.pull_request_commits(&pr).await;

                    self.do_pull_request_comment(&pr, &comment, branch_name, &commits)
                        .await;
                } else if let Some(ref issue) = self.data.issue {
                    // issues do not have branches or commits -> main channel is fine.
                    let branch_name = "";
                    let commits = vec![];

                    self.do_pull_request_comment(&issue, &comment, branch_name, &commits)
                        .await;
                }
            }
        }
        (StatusCode::OK, "issue_comment".into())
    }

    async fn handle_push(&self) -> EventResponse {
        if self.data.deleted() || self.data.created() {
            // ignore
            return (StatusCode::OK, "push [ignored]".into());
        }

        if !self.data.ref_name().is_empty()
            && !self.data.after().is_empty()
            && !self.data.before().is_empty()
        {
            let branch_name = self.data.ref_name().replace("refs/heads/", "");

            let release_branch_prefix = self.config.repos().release_branch_prefix(&self.repository);
            let is_versioned_branch = github::is_main_branch(&branch_name)
                || branch_name.starts_with(&release_branch_prefix);

            // only lookup PRs for non-main branches
            if !is_versioned_branch {
                let prs = match self
                    .github_session
                    .get_pull_requests(
                        self.repository.owner.login(),
                        &self.repository.name,
                        Some("open"),
                        None,
                    )
                    .await
                {
                    Ok(p) => p,
                    Err(e) => {
                        error!(
                            "Error looking up PR for '{}' ({}): {}",
                            branch_name,
                            self.data.after(),
                            e
                        );
                        return (StatusCode::OK, "push [no PR]".into());
                    }
                };

                // there appears to be a race condition in github where the get PR's call may not
                // yet return the new hash, so check both.
                let prs: Vec<github::PullRequest> = prs
                    .into_iter()
                    .filter(|pr| {
                        pr.head.sha == self.data.before() || pr.head.sha == self.data.after()
                    })
                    .collect();

                if prs.is_empty() {
                    info!("No PRs found for '{}' ({})", branch_name, self.data.after());
                } else {
                    let attachments: Vec<_> = if let Some(ref commits) = self.data.commits {
                        commits
                            .iter()
                            .map(|commit| {
                                let msg = commit.message.lines().next().unwrap_or("");
                                let hash: &str = &commit.id[0..7];
                                let attach =
                                    format!("{}: {}", util::make_link(&commit.url, hash), msg);
                                SlackAttachmentBuilder::new(&attach).build()
                            })
                            .collect()
                    } else {
                        vec![]
                    };

                    let message = format!(
                        "{} pushed {} commit(s) to branch {}",
                        self.slack_user_name(&self.data.sender),
                        attachments.len(),
                        branch_name
                    );

                    for pull_request in &prs {
                        if pull_request.is_draft() {
                            info!("Skipping WIP PR #{}", pull_request.number);
                            continue;
                        }

                        let mut attachments = attachments.clone();
                        attachments.insert(
                            0,
                            SlackAttachmentBuilder::new("")
                                .title(format!(
                                    "Pull Request #{}: \"{}\"",
                                    pull_request.number,
                                    pull_request.title.as_str()
                                ))
                                .title_link(pull_request.html_url.as_str())
                                .build(),
                        );

                        let commits = self.pull_request_commits(&pull_request).await;

                        self.messenger.send_to_all(
                            &message,
                            &attachments,
                            &pull_request.user,
                            &self.data.sender,
                            &self.repository,
                            &self.all_participants(&pull_request, &commits).await,
                            &branch_name,
                            &commits,
                            vec![self.build_thread_guid(pull_request.number)],
                        );

                        if self.data.forced()
                            && self.config.repos().notify_force_push(&self.repository)
                        {
                            let msg = force_push::req(
                                &self.repository,
                                pull_request,
                                self.data.before(),
                                self.data.after(),
                            );
                            self.force_push.send(msg);
                        }

                        // Lookup jira projects for this PR's base branch
                        let jira_projects = self
                            .config
                            .repos()
                            .jira_projects(&self.repository, &pull_request.base.ref_name);

                        // Mark if no JIRA references
                        jira::check_jira_refs(
                            pull_request,
                            &commits,
                            &jira_projects,
                            self.github_session.deref(),
                        )
                        .await;
                    }
                }
            }

            // Note: check for jira projects on the branch being pushed to
            let has_jira_projects = !self
                .config
                .repos()
                .jira_projects(&self.repository, &branch_name)
                .is_empty();

            // Mark JIRAs as merged
            if is_versioned_branch && has_jira_projects {
                if let Some(ref commits) = self.data.commits {
                    let msg = repo_version::req(
                        &self.repository,
                        &branch_name,
                        self.data.after(),
                        commits,
                    );
                    self.repo_version.send(msg);
                }
            }
        }

        (StatusCode::OK, "push".into())
    }

    async fn merge_pull_request_all_labels(
        &self,
        pull_request: &github::PullRequest,
        release_branch_prefix: &str,
        commits: &[github::Commit],
    ) {
        if !pull_request.is_merged() {
            return;
        }

        let branch_name = &pull_request.base.ref_name;

        let labels = match self
            .github_session
            .get_pull_request_labels(
                self.repository.owner.login(),
                &self.repository.name,
                pull_request.number,
            )
            .await
        {
            Ok(l) => l,
            Err(e) => {
                self.messenger.send_to_owner(
                    "Error getting Pull Request labels",
                    &[SlackAttachmentBuilder::new(&format!("{}", e))
                        .color("danger")
                        .build()],
                    &pull_request.user,
                    &self.repository,
                    branch_name,
                    commits,
                );
                return;
            }
        };

        for label in &labels {
            self.merge_pull_request(pull_request, label, release_branch_prefix, commits);
        }
    }

    fn merge_pull_request(
        &self,
        pull_request: &github::PullRequest,
        label: &github::Label,
        release_branch_prefix: &str,
        commits: &[github::Commit],
    ) {
        if !pull_request.is_merged() {
            return;
        }

        let re = Regex::new(r"(?i)backport-(.+)").unwrap();
        let backport = match re.captures(&label.name) {
            Some(c) => c[1].to_string(),
            None => return,
        };
        let target_branch = if github::is_main_branch(&backport) {
            backport
        } else {
            release_branch_prefix.to_string() + &backport
        };

        let req = pr_merge::req(
            &self.repository,
            pull_request,
            &target_branch,
            release_branch_prefix,
            commits,
        );
        self.pr_merge.send(req);
    }

    fn build_thread_guid(&self, number: u32) -> String {
        format!(
            "{}/{}/{}",
            self.repository.owner.login(),
            self.repository.name,
            number
        )
    }
}
