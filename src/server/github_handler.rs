use std::ops::Deref;
use std::sync::{Arc, Mutex};

use futures::Future;
use futures::Stream;
use hyper::server::{Request, Response};
use hyper::StatusCode;
use regex::Regex;
use serde_json;

use config::Config;
use github;
use github::CommentLike;
use git_clone_manager::GitCloneManager;
use jira;
use messenger::{self, Messenger};
use repo_version::{self, RepoVersionRequest};
use pr_merge::{self, PRMergeRequest};
use force_push::{self, ForcePushRequest};
use server::http::{FutureResponse, OctobotHandler};
use server::github_verify::GithubWebhookVerifier;
use slack::SlackAttachmentBuilder;
use util;
use worker::{Worker, WorkSender};

pub struct GithubHandler {
    pub config: Arc<Config>,
    pub github_session: Arc<github::api::Session>,
    pub jira_session: Option<Arc<jira::api::Session>>,
    git_clone_manager: Arc<GitCloneManager>,
    pr_merge_worker: Worker<PRMergeRequest>,
    repo_version_worker: Worker<RepoVersionRequest>,
    force_push_worker: Worker<ForcePushRequest>,
    recent_events: Mutex<Vec<String>>,
}

pub struct GithubEventHandler {
    pub messenger: Box<Messenger>,
    pub config: Arc<Config>,
    pub event: String,
    pub data: github::HookBody,
    pub action: String,
    pub github_session: Arc<github::api::Session>,
    pub jira_session: Option<Arc<jira::api::Session>>,
    pub pr_merge: WorkSender<PRMergeRequest>,
    pub repo_version: WorkSender<RepoVersionRequest>,
    pub force_push: WorkSender<ForcePushRequest>,
    pub git_clone_manager: Arc<GitCloneManager>,
}

const MAX_CONCURRENT_MERGES: usize = 20;
const MAX_CONCURRENT_VERSIONS: usize = 20;
const MAX_CONCURRENT_FORCE_PUSH: usize = 20;

impl GithubHandler {
    pub fn new(config: Arc<Config>,
               github_session: Arc<github::api::Session>,
               jira_session: Option<Arc<jira::api::Session>>) -> Box<GithubHandler> {

        let git_clone_manager = Arc::new(GitCloneManager::new(github_session.clone(), config.clone()));

        Box::new(GithubHandler {
            config: config.clone(),
            github_session: github_session.clone(),
            jira_session: jira_session.clone(),
            git_clone_manager: git_clone_manager.clone(),
            pr_merge_worker: pr_merge::new_worker(MAX_CONCURRENT_MERGES,
                                                  config.clone(),
                                                  github_session.clone(),
                                                  git_clone_manager.clone()),
            repo_version_worker: repo_version::new_worker(MAX_CONCURRENT_VERSIONS,
                                                           config.clone(),
                                                           github_session.clone(),
                                                           jira_session.clone(),
                                                           git_clone_manager.clone()),
            force_push_worker: force_push::new_worker(MAX_CONCURRENT_FORCE_PUSH,
                                                      config.clone(),
                                                      github_session.clone(),
                                                      git_clone_manager.clone()),
            recent_events: Mutex::new(Vec::new()),
        })
    }
}

fn check_unique_event(event: String, events: &mut Vec<String>, trim_at: usize, trim_to: usize) -> bool {
    let unique = !events.contains(&event);

    if unique {
        events.push(event);
        if events.len() > trim_at {
            // reverse so that that we keep recent events
            events.reverse();
            events.truncate(trim_to);
            events.reverse();
        }
    }

    unique
}

impl OctobotHandler for GithubHandler {
    fn handle(&self, req: Request) -> FutureResponse {
        let event_id: String = match req.headers().get_raw("x-github-delivery") {
            Some(ref h) if h.len() == 1 => String::from_utf8_lossy(&h[0]).into_owned(),
            None | Some(..) => {
                let msg = "Expected to find exactly one event id header";
                error!("{}", msg);
                return self.respond_with(StatusCode::BadRequest, &msg);
            }
        };
        {
            let mut recent_events = self.recent_events.lock().unwrap();
            if !check_unique_event(event_id.clone(), &mut recent_events, 1000, 100) {
                let msg = format!("Duplicate X-Github-Delivery header: {}", event_id);
                error!("{}", msg);
                return self.respond_with(StatusCode::BadRequest, &msg);
            }
        }

        let event: String = match req.headers().get_raw("x-github-event") {
            Some(ref h) if h.len() == 1 => String::from_utf8_lossy(&h[0]).into_owned(),
            None | Some(..) => {
                error!("Expected to find exactly one event header");
                return self.respond_with(StatusCode::BadRequest,
                                         "Expected to find exactly one event header");
            }
        };

        let headers = req.headers().clone();
        let github_session = self.github_session.clone();
        let config = self.config.clone();
        let git_clone_manager = self.git_clone_manager.clone();
        let jira_session = self.jira_session.clone();
        let pr_merge = self.pr_merge_worker.new_sender();
        let repo_version = self.repo_version_worker.new_sender();
        let force_push = self.force_push_worker.new_sender();

        Box::new(req.body().concat2().map(move |body| {
            let verifier = GithubWebhookVerifier { secret: config.github.webhook_secret.clone() };
            if !verifier.is_req_valid(&headers, &body) {
                return Response::new().with_status(StatusCode::Forbidden).with_body("Invalid signature");
            }

            let mut data: github::HookBody = match serde_json::from_slice(&body) {
                Ok(h) => h,
                Err(e) => {
                    error!("Error parsing json: {}\n---\n{}\n---\n", e, String::from_utf8_lossy(&body));
                    return Response::new()
                        .with_status(StatusCode::BadRequest)
                        .with_body(format!("Error parsing JSON: {}", e));
                }
            };

            let action = match data.action {
                Some(ref a) => a.clone(),
                None => String::new(),
            };

            // refetch PR if present to get requested reviewers: they don't come on each webhook :cry:
            let mut changed_pr = None;
            if let Some(ref pull_request) = data.pull_request {
                if pull_request.requested_reviewers.is_none() {
                    match github_session.get_pull_request(&data.repository.owner.login(),
                                                               &data.repository.name,
                                                               pull_request.number) {
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
                messenger: messenger::from_config(config.clone()),
                github_session: github_session,
                git_clone_manager: git_clone_manager,
                jira_session: jira_session,
                pr_merge: pr_merge,
                repo_version: repo_version,
                force_push: force_push,
            };

            match handler.handle_event() {
                Some((status, resp)) => Response::new().with_status(status).with_body(resp),
                None => Response::new()
                    .with_status(StatusCode::Ok)
                    .with_body(format!("Unhandled event: {}", event)),
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

    fn slack_user_name(&self, user: &github::User) -> String {
        self.config.users().slack_user_name(user.login(), &self.data.repository)
    }

    fn pull_request_commits(&self, pull_request: &github::PullRequestLike) -> Vec<github::Commit> {
        match self.github_session.get_pull_request_commits(&self.data.repository.owner.login(),
                                                           &self.data.repository.name,
                                                           pull_request.number()) {
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

    fn all_participants_with_commits(&self, pull_request: &github::PullRequestLike, pr_commits: &Vec<github::Commit>) -> Vec<github::User> {
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
        (StatusCode::Ok, "ping".into())
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
                let assignees_str = self.config
                    .users()
                    .slack_user_names(&pull_request.assignees, &self.data.repository)
                    .join(", ");
                verb = Some(format!("assigned to {}", assignees_str));
                notify_channel_only = false;
            } else if self.action == "unassigned" {
                verb = Some("unassigned".to_string());
                notify_channel_only = true;
            } else if self.action == "review_requested" {
                if let Some(ref reviewers) = pull_request.requested_reviewers {
                    let assignees_str = self.config
                        .users()
                        .slack_user_names(reviewers, &self.data.repository)
                        .join(", ");
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

                if !pull_request.is_wip() {
                    let msg = format!("Pull Request {}", verb);
                    let attachments = vec![SlackAttachmentBuilder::new("")
                                               .title(format!("Pull Request #{}: \"{}\"",
                                                              pull_request.number,
                                                              pull_request.title.as_str()))
                                               .title_link(pull_request.html_url.as_str())
                                               .build()];

                    if notify_channel_only {
                        self.messenger.send_to_channel(&msg,
                                                       &attachments,
                                                       &self.data.repository);
                    } else {
                        self.messenger.send_to_all(&msg,
                                                   &attachments,
                                                   &pull_request.user,
                                                   &self.data.sender,
                                                   &self.data.repository,
                                                   &self.all_participants_with_commits(&pull_request, &commits));

                    }
                }

                // Mark JIRAs in review for PR open
                if self.action == "opened" {
                    if let Some(ref jira_config) = self.config.jira {
                        if let Some(ref jira_session) = self.jira_session {
                            let jira_projects = self.config.repos().jira_projects(&self.data.repository, &pull_request.base.ref_name);

                            jira::workflow::submit_for_review(&pull_request,
                                                              &commits,
                                                              &jira_projects,
                                                              jira_session.deref(),
                                                              jira_config);
                        }
                    }
                }
            }

            if self.action == "labeled" {
                if let Some(ref label) = self.data.label {
                    self.merge_pull_request(pull_request, label);
                }
            } else if verb == Some("merged".to_string()) {
                self.merge_pull_request_all_labels(pull_request);
            }
        }

        (StatusCode::Ok, "pr".into())
    }

    fn handle_pr_review_comment(&self) -> EventResponse {
        if let Some(ref pull_request) = self.data.pull_request {
            if let Some(ref comment) = self.data.comment {
                if self.action == "created" {
                    self.do_pull_request_comment(&pull_request, &comment)
                }

            }
        }

        (StatusCode::Ok, "pr_review_comment".into())
    }

    fn handle_pr_review(&self) -> EventResponse {
        if let Some(ref pull_request) = self.data.pull_request {
            if let Some(ref review) = self.data.review {
                if self.action == "submitted" {

                    // just a comment. should just be handled by regular comment handler.
                    if review.state == "commented" {
                        self.do_pull_request_comment(&pull_request, &review);
                        return (StatusCode::Ok, "pr_review [comment]".into());
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
                        return (StatusCode::Ok, "pr_review [ignored]".into());
                    }

                    let msg = format!("{} {} PR \"{}\"",
                                      self.slack_user_name(&review.user),
                                      action_msg,
                                      util::make_link(pull_request.html_url.as_str(),
                                                      pull_request.title.as_str()));

                    let attachments = vec![SlackAttachmentBuilder::new(review.body())
                                               .title(format!("Review: {}", state_msg))
                                               .title_link(review.html_url.as_str())
                                               .color(color)
                                               .build()];

                    let mut participants = self.all_participants(&pull_request);
                    for username in util::get_mentioned_usernames(review.body()) {
                        participants.push(github::User::new(username))
                    }

                    self.messenger.send_to_all(&msg,
                                               &attachments,
                                               &pull_request.user,
                                               &self.data.sender,
                                               &self.data.repository,
                                               &participants);
                }
            }
        }

        (StatusCode::Ok, "pr_review".into())
    }

    fn do_pull_request_comment(&self, pull_request: &github::PullRequestLike,
                               comment: &github::CommentLike) {
        if comment.body().trim().len() == 0 {
            return;
        }

        if comment.user().login() == self.github_session.user().login() {
            info!("Ignoring message from octobot ({}): {}",
                  self.github_session.user().login(),
                  comment.body());
            return;
        }

        let msg = format!("Comment on \"{}\"",
                          util::make_link(pull_request.html_url(), pull_request.title()));

        let attachments = vec![SlackAttachmentBuilder::new(comment.body().trim())
                                   .title(format!("{} said:",
                                                  self.slack_user_name(comment.user())))
                                   .title_link(comment.html_url())
                                   .build()];

        let mut participants = self.all_participants(pull_request);
        for username in util::get_mentioned_usernames(comment.body()) {
            participants.push(github::User::new(username))
        }

        self.messenger.send_to_all(&msg,
                                   &attachments,
                                   pull_request.user(),
                                   &self.data.sender,
                                   &self.data.repository,
                                   &participants);

    }

    fn handle_commit_comment(&self) -> EventResponse {
        if let Some(ref comment) = self.data.comment {
            if self.action == "created" {
                if let Some(ref commit_id) = comment.commit_id {
                    let commit: &str = &commit_id[0..7];
                    let commit_url =
                        format!("{}/commit/{}", self.data.repository.html_url, commit_id);
                    let commit_path: String;
                    if let Some(ref path) = comment.path {
                        commit_path = path.to_string();
                    } else {
                        commit_path = commit.to_string();
                    }

                    let msg = format!("Comment on \"{}\" ({})",
                                      commit_path,
                                      util::make_link(commit_url.as_str(), commit));

                    let attachments = vec![SlackAttachmentBuilder::new(comment.body())
                                               .title(format!("{} said:",
                                                              self.slack_user_name(&comment.user)))
                                               .title_link(comment.html_url.as_str())
                                               .build()];

                    self.messenger.send_to_all(&msg,
                                               &attachments,
                                               &comment.user,
                                               &self.data.sender,
                                               &self.data.repository,
                                               &vec![]);
                }
            }
        }

        (StatusCode::Ok, "commit_comment".into())
    }

    fn handle_issue_comment(&self) -> EventResponse {
        if let Some(ref issue) = self.data.issue {
            if let Some(ref comment) = self.data.comment {
                if self.action == "created" {
                    self.do_pull_request_comment(&issue, &comment);
                }
            }
        }
        (StatusCode::Ok, "issue_comment".into())
    }

    fn handle_push(&self) -> EventResponse {
        if self.data.deleted() || self.data.created() {
            // ignore
            return (StatusCode::Ok, "push [ignored]".into());
        }
        if self.data.ref_name().len() > 0 && self.data.after().len() > 0 &&
           self.data.before().len() > 0 {

            let branch_name = self.data.ref_name().replace("refs/heads/", "");

            let is_main_branch = branch_name == "master" || branch_name == "develop" || branch_name.starts_with("release");

            // only lookup PRs for non-main branches
            if !is_main_branch {
                let prs = match self.github_session
                    .get_pull_requests(&self.data.repository.owner.login(),
                                       &self.data.repository.name,
                                       Some("open"),
                                       None) {
                    Ok(p) => p,
                    Err(e) => {
                        error!("Error looking up PR for '{}' ({}): {}",
                               branch_name,
                               self.data.after(),
                               e);
                        return (StatusCode::Ok, "push [no PR]".into());
                    }
                };

                // there appears to be a race condition in github where the get PR's call may not
                // yet return the new hash, so check both.
                let prs: Vec<github::PullRequest> =
                    prs.into_iter()
                       .filter(|pr| pr.head.sha == self.data.before() || pr.head.sha == self.data.after())
                       .collect();

                if prs.len() == 0 {
                    info!("No PRs found for '{}' ({})", branch_name, self.data.after());
                } else {
                    let attachments: Vec<_>;
                    if let Some(ref commits) = self.data.commits {
                        attachments = commits.iter()
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

                    let message = format!("{} pushed {} commit(s) to branch {}",
                                          self.slack_user_name(&self.data.sender),
                                          attachments.len(),
                                          branch_name);

                    for pull_request in &prs {
                        if pull_request.is_wip() {
                            info!("Skipping WIP PR #{}", pull_request.number);
                            continue;
                        }

                        let mut attachments = attachments.clone();
                        attachments.insert(0,
                                           SlackAttachmentBuilder::new("")
                                               .title(format!("Pull Request #{}: \"{}\"",
                                                              pull_request.number,
                                                              pull_request.title.as_str()))
                                               .title_link(pull_request.html_url.as_str())
                                               .build());

                        self.messenger.send_to_all(&message,
                                                   &attachments,
                                                   &pull_request.user,
                                                   &self.data.sender,
                                                   &self.data.repository,
                                                   &self.all_participants(&pull_request));

                        if self.data.forced() &&
                           self.config.repos().notify_force_push(&self.data.repository) {
                            let msg = force_push::req(&self.data.repository, pull_request, self.data.before(), self.data.after());
                            if let Err(e) = self.force_push.send(msg) {
                                error!("Error sending force push message: {}", e);
                            }
                        }
                    }
                }
            }

            // Mark JIRAs as merged
            if is_main_branch {
                if let Some(ref jira_config) = self.config.jira {
                    if let Some(ref jira_session) = self.jira_session {
                        if let Some(ref commits) = self.data.commits {

                            // try to send resolve message w/ a version in it if possible
                            let has_version = match self.config.repos().version_script(&self.data.repository, &branch_name) {
                                Some(_) => {
                                    let msg = repo_version::req(&self.data.repository, &branch_name, self.data.after(), commits);
                                    if let Err(e) = self.repo_version.send(msg) {
                                        error!("Error sending version request message: {}", e);
                                        false
                                    } else {
                                        true
                                    }
                                },
                                None => false,
                            };
                            if !has_version {
                                jira::workflow::resolve_issue(&branch_name,
                                                              None,
                                                              commits,
                                                              &self.config.repos().jira_projects(&self.data.repository, &branch_name),
                                                              jira_session.deref(), jira_config);
                            }
                        }
                    }
                }
            }
        }

        (StatusCode::Ok, "push".into())
    }

    fn merge_pull_request_all_labels(&self, pull_request: &github::PullRequest) {
        if !pull_request.is_merged() {
            return;
        }

        let labels = match self.github_session
            .get_pull_request_labels(&self.data.repository.owner.login(),
                                     &self.data.repository.name,
                                     pull_request.number) {
            Ok(l) => l,
            Err(e) => {
                self.messenger.send_to_owner("Error getting Pull Request labels",
                                             &vec![SlackAttachmentBuilder::new(&e)
                                                       .color("danger")
                                                       .build()],
                                             &pull_request.user,
                                             &self.data.repository);
                return;
            }
        };

        for label in &labels {
            self.merge_pull_request(pull_request, label);
        }
    }

    fn merge_pull_request(&self, pull_request: &github::PullRequest, label: &github::Label) {
        if !pull_request.is_merged() {
            return;
        }

        let re = Regex::new(r"(?i)backport-([\d\.]+)").unwrap();
        let backport = match re.captures(&label.name) {
            Some(c) => c[1].to_string(),
            None => return,
        };
        let target_branch = "release/".to_string() + &backport;

        let req = pr_merge::req(&self.data.repository, pull_request, &target_branch);
        if let Err(e) = self.pr_merge.send(req) {
            error!("Error sending merge request message: {}", e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_unique_event() {
        let trim_at = 5;
        let trim_to = 2;
        let mut events = vec![];

        assert!(check_unique_event("A".into(), &mut events, trim_at, trim_to));
        assert_eq!(vec!["A"], events);

        assert!(check_unique_event("B".into(), &mut events, trim_at, trim_to));
        assert_eq!(vec!["A", "B"], events);
        assert!(!check_unique_event("B".into(), &mut events, trim_at, trim_to));
        assert_eq!(vec!["A", "B"], events);

        assert!(check_unique_event("C".into(), &mut events, trim_at, trim_to));
        assert!(check_unique_event("D".into(), &mut events, trim_at, trim_to));
        assert!(check_unique_event("E".into(), &mut events, trim_at, trim_to));
        assert_eq!(vec!["A", "B", "C", "D", "E"], events);

        // next one should trigger a trim!
        assert!(check_unique_event("F".into(), &mut events, trim_at, trim_to));
        assert_eq!(vec!["E", "F"], events);
    }
}
