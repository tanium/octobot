use std::ops::Deref;
use std::sync::{Arc, Mutex};

use futures::Future;
use futures::Stream;
use hyper::{Body, Request, StatusCode};
use regex::Regex;
use serde_json;
use tokio;

use config::Config;
use force_push::{self, ForcePushRequest};
use git_clone_manager::GitCloneManager;
use github;
use github::CommentLike;
use github::api::Session;
use jira;
use messenger::{self, Messenger};
use pr_merge::{self, PRMergeRequest};
use repo_version::{self, RepoVersionRequest};
use runtime;
use server::github_verify::GithubWebhookVerifier;
use server::http::{FutureResponse, Handler};
use slack::{self, SlackAttachmentBuilder, SlackRequest};
use util;
use worker::{Worker, TokioWorker};

pub struct GithubHandlerState {
    pub config: Arc<Config>,
    pub github_app: Arc<github::api::GithubSessionFactory>,
    pub jira_session: Option<Arc<jira::api::Session>>,
    _runtime: Arc<Mutex<tokio::runtime::Runtime>>,
    pr_merge_worker: Arc<Worker<PRMergeRequest>>,
    repo_version_worker: Arc<Worker<RepoVersionRequest>>,
    force_push_worker: Arc<Worker<ForcePushRequest>>,
    slack_worker: Arc<Worker<SlackRequest>>,
    recent_events: Mutex<Vec<String>>,
}

pub struct GithubHandler {
    state: Arc<GithubHandlerState>,
}

pub struct GithubEventHandler {
    pub messenger: Box<Messenger>,
    pub config: Arc<Config>,
    pub event: String,
    pub data: github::HookBody,
    pub action: String,
    pub github_session: Arc<github::api::Session>,
    pub jira_session: Option<Arc<jira::api::Session>>,
    pub pr_merge: Arc<Worker<PRMergeRequest>>,
    pub repo_version: Arc<Worker<RepoVersionRequest>>,
    pub force_push: Arc<Worker<ForcePushRequest>>,
}

const MAX_CONCURRENT_JOBS: usize = 20;
const MAX_COMMITS_FOR_JIRA_CONSIDERATION: usize = 20;

impl GithubHandlerState {
    pub fn new(
        config: Arc<Config>,
        github_app: Arc<github::api::GithubSessionFactory>,
        jira_session: Option<Arc<jira::api::Session>>,
    ) -> GithubHandlerState {

        let git_clone_manager = Arc::new(GitCloneManager::new(github_app.clone(), config.clone()));

        let runtime = Arc::new(Mutex::new(runtime::new(MAX_CONCURRENT_JOBS, "jobs")));

        let slack_worker = TokioWorker::new(runtime.clone(), slack::new_runner(&config.main.slack_webhook_url));
        let pr_merge_worker = TokioWorker::new(runtime.clone(), pr_merge::new_runner(
            config.clone(),
            github_app.clone(),
            git_clone_manager.clone(),
            slack_worker.clone(),
        ));
        let repo_version_worker = TokioWorker::new(runtime.clone(), repo_version::new_runner(
            config.clone(),
            github_app.clone(),
            jira_session.clone(),
            git_clone_manager.clone(),
            slack_worker.clone(),
        ));
        let force_push_worker = TokioWorker::new(runtime.clone(), force_push::new_runner(
            config.clone(),
            github_app.clone(),
            git_clone_manager.clone(),
        ));

        GithubHandlerState {
            config: config.clone(),
            github_app: github_app.clone(),
            jira_session: jira_session.clone(),
            _runtime: runtime,
            pr_merge_worker: pr_merge_worker,
            repo_version_worker: repo_version_worker,
            force_push_worker: force_push_worker,
            slack_worker: slack_worker,
            recent_events: Mutex::new(Vec::new()),
        }
    }
}

impl GithubHandler {
    pub fn new(
        config: Arc<Config>,
        github_app: Arc<github::api::GithubSessionFactory>,
        jira_session: Option<Arc<jira::api::Session>>,
    ) -> Box<GithubHandler> {
        let state = GithubHandlerState::new(config, github_app, jira_session);
        GithubHandler::from_state(Arc::new(state))
    }

    pub fn from_state(state: Arc<GithubHandlerState>) -> Box<GithubHandler> {
        Box::new(GithubHandler { state: state })
    }
}

impl Handler for GithubHandler {
    fn handle(&self, req: Request<Body>) -> FutureResponse {
        let event_id;
        {
            let values = req.headers().get_all("x-github-delivery").iter().collect::<Vec<_>>();
            if values.len() != 1 {
                let msg = "Expected to find exactly one event id header";
                error!("{}", msg);
                return self.respond(util::new_bad_req_resp(msg));
            }

            event_id = String::from_utf8_lossy(values[0].as_bytes()).into_owned();
        }

        // make sure event id is valid
        {
            let mut recent_events = self.state.recent_events.lock().unwrap();
            if !util::check_unique_event(event_id.clone(), &mut *recent_events, 1000, 100) {
                let msg = format!("Duplicate X-Github-Delivery header: {}", event_id);
                error!("{}", msg);
                return self.respond(util::new_bad_req_resp(msg));
            }
        }

        let event;
        {
            let values = req.headers().get_all("x-github-event").iter().collect::<Vec<_>>();
            if values.len() != 1 {
                let msg = "Expected to find exactly one event header";
                error!("{}", msg);
                return self.respond(util::new_bad_req_resp(msg));
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

        Box::new(req.into_body().concat2().map(move |body| {
            let verifier = GithubWebhookVerifier { secret: config.github.webhook_secret.clone() };
            if !verifier.is_req_valid(&headers, &body) {
                return util::new_msg_resp(StatusCode::FORBIDDEN, "Invalid signature");
            }

            let mut data: github::HookBody = match serde_json::from_slice(&body) {
                Ok(h) => h,
                Err(e) => {
                    error!("Error parsing json: {}\n---\n{}\n---\n", e, String::from_utf8_lossy(&body));
                    return util::new_bad_req_resp(format!("Error parsing JSON: {}", e));
                }
            };

            let github_session = match github_app.new_session(&data.repository.owner.login(), &data.repository.name) {
                // Note: this doesn't really need to be an Arc anymore...
                Ok(g) => Arc::new(g),
                Err(e) => {
                    error!(
                        "Error creating a new github session for {}/{}: {}",
                        data.repository.owner.login(),
                        &data.repository.name,
                        e
                    );
                    return util::new_bad_req_resp("Could not create github session");
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
                    data.pull_request = match github_session.get_pull_request(
                        &data.repository.owner.login(),
                        &data.repository.name,
                        issue.number,
                    ) {
                        Ok(pr) => Some(pr),
                        Err(e) => {
                            error!("Error refetching issue #{} as pull request: {}", issue.number, e);
                            None
                        }
                    };
                }
            }

            // refetch PR if present to get requested reviewers: they don't come on each webhook :cry:
            let mut changed_pr = None;
            if let Some(ref pull_request) = data.pull_request {
                if pull_request.requested_reviewers.is_none() || pull_request.reviews.is_none() {
                    match github_session.get_pull_request(
                        &data.repository.owner.login(),
                        &data.repository.name,
                        pull_request.number,
                    ) {
                        Ok(pr) => changed_pr = Some(pr),
                        Err(e) => error!("Error refetching pull request to get reviewers: {}", e),
                    };
                }
            }
            if let Some(changed_pr) = changed_pr {
                data.pull_request = Some(changed_pr);
            }

            let handler = GithubEventHandler {
                event: event.clone(),
                data: data,
                action: action,
                config: config.clone(),
                messenger: Box::new(messenger::new(config.clone(), slack)),
                github_session: github_session,
                jira_session: jira_session,
                pr_merge: pr_merge,
                repo_version: repo_version,
                force_push: force_push,
            };

            match handler.handle_event() {
                Some((status, resp)) => util::new_msg_resp(status, resp),
                None => util::new_msg_resp(StatusCode::OK, format!("Unhandled event: {}", event)),
            }
        }))
    }
}

type EventResponse = (StatusCode, String);

impl GithubEventHandler {
    pub fn handle_event(&self) -> Option<EventResponse> {
        info!("Received event: {}", self.event);
        if self.event == "ping" {
            Some(self.handle_ping())
        } else if self.event == "pull_request" {
            Some(self.handle_pr())
        } else if self.event == "pull_request_review_comment" {
            Some(self.handle_pr_review_comment())
        } else if self.event == "pull_request_review" {
            Some(self.handle_pr_review())
        } else if self.event == "commit_comment" {
            Some(self.handle_commit_comment())
        } else if self.event == "issue_comment" {
            Some(self.handle_issue_comment())
        } else if self.event == "push" {
            Some(self.handle_push())
        } else {
            None
        }
    }

    // This defaults to using the github name if no slack name is configured, since this is not
    // used for actually sending messages, but just for referring to users in slack messages.
    fn slack_user_name(&self, user: &github::User) -> String {
        match self.config.users().slack_user_name(&user.login()) {
            Some(slack_user) => slack_user.to_string(),
            None => user.login().to_string(),
        }
    }

    fn slack_user_names(&self, users: &Vec<github::User>) -> Vec<String> {
        users.iter().map(|u| self.slack_user_name(u)).collect()
    }

    fn pull_request_commits(&self, pull_request: &github::PullRequestLike) -> Vec<github::Commit> {
        if !pull_request.has_commits() {
            return vec![];
        }

        match self.github_session.get_pull_request_commits(
            &self.data.repository.owner.login(),
            &self.data.repository.name,
            pull_request.number(),
        ) {
            Ok(commits) => commits,
            Err(e) => {
                error!("Error looking up PR commits: {}", e);
                vec![]
            }
        }
    }

    fn all_participants(&self, pull_request: &github::PullRequestLike) -> Vec<github::User> {
        self.all_participants_with_commits(pull_request, &self.pull_request_commits(pull_request))
    }

    fn all_participants_with_commits(
        &self,
        pull_request: &github::PullRequestLike,
        pr_commits: &Vec<github::Commit>,
    ) -> Vec<github::User> {
        // start with the assignees
        let mut participants = pull_request.assignees().clone();
        // add the author of the PR
        participants.push(pull_request.user().clone());
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

    fn handle_ping(&self) -> EventResponse {
        (StatusCode::OK, "ping".into())
    }

    fn handle_pr(&self) -> EventResponse {
        if let Some(ref pull_request) = self.data.pull_request {
            let verb: Option<String>;
            let notify_channel_only;
            if self.action == "opened" {
                verb = Some(format!("opened by {}", self.slack_user_name(&pull_request.user)));
                notify_channel_only = true;
            } else if self.action == "closed" {
                if pull_request.merged == Some(true) {
                    verb = Some("merged".to_string());
                } else {
                    verb = Some("closed".to_string());
                }
                notify_channel_only = false;
            } else if self.action == "reopened" {
                verb = Some("reopened".to_string());
                notify_channel_only = true;
            } else if self.action == "assigned" {
                let assignees_str = self.slack_user_names(&pull_request.assignees).join(", ");
                verb = Some(format!("assigned to {}", assignees_str));
                notify_channel_only = false;
            } else if self.action == "unassigned" {
                verb = Some("unassigned".to_string());
                notify_channel_only = true;
            } else if self.action == "review_requested" {
                if let Some(ref reviewers) = pull_request.requested_reviewers {
                    let assignees_str = self.slack_user_names(reviewers).join(", ");
                    verb = Some(format!("submitted for review to {}", assignees_str));
                } else {
                    verb = None;
                }
                notify_channel_only = false;
            } else {
                verb = None;
                notify_channel_only = true;
            }

            if let Some(ref verb) = verb {
                let commits = self.pull_request_commits(&pull_request);

                let attachments = vec![
                    SlackAttachmentBuilder::new("")
                        .title(format!("Pull Request #{}: \"{}\"", pull_request.number, pull_request.title.as_str()))
                        .title_link(pull_request.html_url.as_str())
                        .build(),
                ];

                if !pull_request.is_wip() {
                    let msg = format!("Pull Request {}", verb);

                    if notify_channel_only {
                        self.messenger.send_to_channel(&msg, &attachments, &self.data.repository);
                    } else {
                        self.messenger.send_to_all(
                            &msg,
                            &attachments,
                            &pull_request.user,
                            &self.data.sender,
                            &self.data.repository,
                            &self.all_participants_with_commits(&pull_request, &commits),
                        );

                    }
                }

                // Mark JIRAs in review for PR open
                if self.action == "opened" {
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
                                    &self.data.repository,
                                );

                            } else {
                                let jira_projects = self.config.repos().jira_projects(
                                    &self.data.repository,
                                    &pull_request.base.ref_name,
                                );

                                jira::workflow::submit_for_review(
                                    &pull_request,
                                    &commits,
                                    &jira_projects,
                                    jira_session.deref(),
                                    jira_config,
                                );
                            }
                        }
                    }
                }
            }

            let release_branch_prefix = self.config.repos().release_branch_prefix(
                &self.data.repository,
                &pull_request.base.ref_name,
            );
            if self.action == "labeled" {
                if let Some(ref label) = self.data.label {
                    self.merge_pull_request(pull_request, label, &release_branch_prefix);
                }
            } else if verb == Some("merged".to_string()) {
                self.merge_pull_request_all_labels(pull_request, &release_branch_prefix);
            }
        }

        (StatusCode::OK, "pr".into())
    }

    fn handle_pr_review_comment(&self) -> EventResponse {
        if let Some(ref pull_request) = self.data.pull_request {
            if let Some(ref comment) = self.data.comment {
                if self.action == "created" {
                    self.do_pull_request_comment(&pull_request, &comment)
                }

            }
        }

        (StatusCode::OK, "pr_review_comment".into())
    }

    fn handle_pr_review(&self) -> EventResponse {
        if let Some(ref pull_request) = self.data.pull_request {
            if let Some(ref review) = self.data.review {
                if self.action == "submitted" {

                    // just a comment. should just be handled by regular comment handler.
                    if review.state == "commented" {
                        self.do_pull_request_comment(&pull_request, &review);
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
                        util::make_link(pull_request.html_url.as_str(), pull_request.title.as_str())
                    );

                    let attachments = vec![
                        SlackAttachmentBuilder::new(review.body())
                            .title(format!("Review: {}", state_msg))
                            .title_link(review.html_url.as_str())
                            .color(color)
                            .build(),
                    ];

                    let mut participants = self.all_participants(&pull_request);
                    for username in util::get_mentioned_usernames(review.body()) {
                        participants.push(github::User::new(username))
                    }

                    self.messenger.send_to_all(
                        &msg,
                        &attachments,
                        &pull_request.user,
                        &self.data.sender,
                        &self.data.repository,
                        &participants,
                    );
                }
            }
        }

        (StatusCode::OK, "pr_review".into())
    }

    fn do_pull_request_comment(&self, pull_request: &github::PullRequestLike, comment: &github::CommentLike) {
        if comment.body().trim().len() == 0 {
            return;
        }

        if comment.user().login() == self.github_session.bot_name() {
            info!("Ignoring message from octobot ({}): {}", self.github_session.bot_name(), comment.body());
            return;
        }

        let msg = format!("Comment on \"{}\"", util::make_link(pull_request.html_url(), pull_request.title()));

        let attachments = vec![
            SlackAttachmentBuilder::new(comment.body().trim())
                .title(format!("{} said:", self.slack_user_name(comment.user())))
                .title_link(comment.html_url())
                .build(),
        ];

        let mut participants = self.all_participants(pull_request);
        for username in util::get_mentioned_usernames(comment.body()) {
            participants.push(github::User::new(username))
        }

        self.messenger.send_to_all(
            &msg,
            &attachments,
            pull_request.user(),
            &self.data.sender,
            &self.data.repository,
            &participants,
        );

    }

    fn handle_commit_comment(&self) -> EventResponse {
        if let Some(ref comment) = self.data.comment {
            if self.action == "created" {
                if let Some(ref commit_id) = comment.commit_id {
                    let commit: &str = &commit_id[0..7];
                    let commit_url = format!("{}/commit/{}", self.data.repository.html_url, commit_id);
                    let commit_path: String;
                    if let Some(ref path) = comment.path {
                        commit_path = path.to_string();
                    } else {
                        commit_path = commit.to_string();
                    }

                    let msg =
                        format!("Comment on \"{}\" ({})", commit_path, util::make_link(commit_url.as_str(), commit));

                    let attachments = vec![
                        SlackAttachmentBuilder::new(comment.body())
                            .title(format!("{} said:", self.slack_user_name(&comment.user)))
                            .title_link(comment.html_url.as_str())
                            .build(),
                    ];

                    self.messenger.send_to_all(
                        &msg,
                        &attachments,
                        &comment.user,
                        &self.data.sender,
                        &self.data.repository,
                        &vec![],
                    );
                }
            }
        }

        (StatusCode::OK, "commit_comment".into())
    }

    fn handle_issue_comment(&self) -> EventResponse {
        if let Some(ref comment) = self.data.comment {
            if self.action == "created" {
                // Check to see if we remapped this "issue" to a PR
                if let Some(ref pr) = self.data.pull_request {
                    self.do_pull_request_comment(&pr, &comment);
                } else if let Some(ref issue) = self.data.issue {
                    self.do_pull_request_comment(&issue, &comment);
                }
            }
        }
        (StatusCode::OK, "issue_comment".into())
    }

    fn handle_push(&self) -> EventResponse {
        if self.data.deleted() || self.data.created() {
            // ignore
            return (StatusCode::OK, "push [ignored]".into());
        }
        if self.data.ref_name().len() > 0 && self.data.after().len() > 0 && self.data.before().len() > 0 {

            let branch_name = self.data.ref_name().replace("refs/heads/", "");

            let release_branch_prefix = self.config.repos().release_branch_prefix(
                &self.data.repository,
                &branch_name
            );
            let next_branch_suffix = self.config.repos().next_branch_suffix(&self.data.repository);

            let is_main_branch = branch_name == "master" || branch_name == "develop" ||
                branch_name.starts_with(&release_branch_prefix);

            let is_next_branch = branch_name.starts_with(&release_branch_prefix) &&
                branch_name.ends_with(&next_branch_suffix);

            // only lookup PRs for non-main branches
            if !is_main_branch {
                let prs = match self.github_session.get_pull_requests(
                    &self.data.repository.owner.login(),
                    &self.data.repository.name,
                    Some("open"),
                    None,
                ) {
                    Ok(p) => p,
                    Err(e) => {
                        error!("Error looking up PR for '{}' ({}): {}", branch_name, self.data.after(), e);
                        return (StatusCode::OK, "push [no PR]".into());
                    }
                };

                // there appears to be a race condition in github where the get PR's call may not
                // yet return the new hash, so check both.
                let prs: Vec<github::PullRequest> = prs.into_iter()
                    .filter(|pr| pr.head.sha == self.data.before() || pr.head.sha == self.data.after())
                    .collect();

                if prs.len() == 0 {
                    info!("No PRs found for '{}' ({})", branch_name, self.data.after());
                } else {
                    let attachments: Vec<_>;
                    if let Some(ref commits) = self.data.commits {
                        attachments = commits
                            .iter()
                            .map(|commit| {
                                let msg = commit.message.lines().next().unwrap_or("");
                                let hash: &str = &commit.id[0..7];
                                let attach = format!("{}: {}", util::make_link(&commit.url, hash), msg);
                                SlackAttachmentBuilder::new(&attach).build()
                            })
                            .collect();
                    } else {
                        attachments = vec![];
                    }

                    let message = format!(
                        "{} pushed {} commit(s) to branch {}",
                        self.slack_user_name(&self.data.sender),
                        attachments.len(),
                        branch_name
                    );

                    for pull_request in &prs {
                        if pull_request.is_wip() {
                            info!("Skipping WIP PR #{}", pull_request.number);
                            continue;
                        }

                        let mut attachments = attachments.clone();
                        attachments
                            .insert(
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

                        self.messenger.send_to_all(
                            &message,
                            &attachments,
                            &pull_request.user,
                            &self.data.sender,
                            &self.data.repository,
                            &self.all_participants(&pull_request),
                        );

                        if self.data.forced() && self.config.repos().notify_force_push(&self.data.repository) {
                            let msg = force_push::req(
                                &self.data.repository,
                                pull_request,
                                self.data.before(),
                                self.data.after(),
                            );
                            self.force_push.send(msg);
                        }
                    }
                }
            }

            // Mark JIRAs as merged
            if is_main_branch && !is_next_branch {
                if let Some(ref jira_config) = self.config.jira {
                    if let Some(ref jira_session) = self.jira_session {
                        if let Some(ref commits) = self.data.commits {

                            // try to send resolve message w/ a version in it if possible
                            let has_version =
                                match self.config.repos().version_script(&self.data.repository, &branch_name) {
                                    Some(_) => {
                                        let msg = repo_version::req(
                                            &self.data.repository,
                                            &branch_name,
                                            self.data.after(),
                                            commits,
                                        );
                                        self.repo_version.send(msg);
                                        true
                                    }
                                    None => false,
                                };
                            if !has_version {
                                jira::workflow::resolve_issue(
                                    &branch_name,
                                    None,
                                    commits,
                                    &self.config.repos().jira_projects(&self.data.repository, &branch_name),
                                    jira_session.deref(),
                                    jira_config,
                                );
                            }
                        }
                    }
                }
            }
        }

        (StatusCode::OK, "push".into())
    }

    fn merge_pull_request_all_labels(&self, pull_request: &github::PullRequest, release_branch_prefix: &str) {
        if !pull_request.is_merged() {
            return;
        }

        let labels = match self.github_session.get_pull_request_labels(
            &self.data.repository.owner.login(),
            &self.data.repository.name,
            pull_request.number,
        ) {
            Ok(l) => l,
            Err(e) => {
                self.messenger.send_to_owner(
                    "Error getting Pull Request labels",
                    &vec![SlackAttachmentBuilder::new(&format!("{}", e)).color("danger").build()],
                    &pull_request.user,
                    &self.data.repository,
                );
                return;
            }
        };

        for label in &labels {
            self.merge_pull_request(pull_request, label, release_branch_prefix);
        }
    }

    fn merge_pull_request(
        &self,
        pull_request: &github::PullRequest,
        label: &github::Label,
        release_branch_prefix: &str,
    ) {
        if !pull_request.is_merged() {
            return;
        }

        let re = Regex::new(r"(?i)backport-(.+)").unwrap();
        let backport = match re.captures(&label.name) {
            Some(c) => c[1].to_string(),
            None => return,
        };
        let target_branch = if backport == "master" || backport == "develop" {
            backport
        } else {
            release_branch_prefix.to_string() + &backport
        };

        let req = pr_merge::req(&self.data.repository, pull_request, &target_branch);
        self.pr_merge.send(req);
    }
}
