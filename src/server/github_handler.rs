use super::*;
use std::sync::Arc;
use std::sync::mpsc::Sender;

use super::iron::prelude::*;
use super::iron::status;
use super::iron::middleware::Handler;
use super::bodyparser;
use super::super::rustc_serialize::json;
use super::super::regex::Regex;

use super::super::github;
use super::super::messenger::{self, Messenger};
use super::super::pr_merge::{self, PRMergeMessage};
use super::super::slack::SlackAttachmentBuilder;
use super::super::util;

pub struct GithubHandler {
    pub config: Arc<Config>,
    pub github_session: Arc<github::api::Session>,
    pr_merge_worker: pr_merge::Worker,
}

struct GithubEventHandler {
    pub messenger: Box<Messenger>,
    pub config: Arc<Config>,
    pub event: String,
    pub data: github::HookBody,
    pub action: String,
    pub github_session: Arc<github::api::Session>,
    pub pr_merge: Sender<PRMergeMessage>,
}

const MAX_CONCURRENT_MERGES: usize = 20;

impl GithubHandler {
    pub fn new(config: Arc<Config>, github_session: github::api::Session) -> GithubHandler {

        let session = Arc::new(github_session);
        GithubHandler {
            config: config.clone(),
            github_session: session.clone(),
            pr_merge_worker: pr_merge::Worker::new(MAX_CONCURRENT_MERGES,
                                                   config.clone(),
                                                   session.clone()),
        }
    }
}

impl Handler for GithubHandler {
    fn handle(&self, req: &mut Request) -> IronResult<Response> {
        let event: String = match req.headers.get_raw("x-github-event") {
            Some(ref h) if h.len() == 1 => String::from_utf8_lossy(&h[0]).into_owned(),
            None | Some(..) => {
                error!("Expected to find exactly one event header");
                return Ok(Response::with((status::BadRequest,
                                          "Expected to find exactly one event header")));
            }
        };

        let body = match req.get::<bodyparser::Raw>() {
            Ok(Some(j)) => j,
            Err(_) | Ok(None) => {
                error!("Error reading json");
                return Ok(Response::with((status::BadRequest, format!("Error reading json"))));
            }
        };

        let data: github::HookBody = match json::decode(body.as_str()) {
            Ok(h) => h,
            Err(e) => {
                error!("Error parsing json: {}\n---\n{}\n---\n", e, body.as_str());
                return Ok(Response::with((status::BadRequest,
                                          format!("Error parsing JSON: {}", e))));
            }
        };

        let action = match data.action {
            Some(ref a) => a.clone(),
            None => String::new(),
        };

        let handler = GithubEventHandler {
            event: event.clone(),
            data: data,
            action: action,
            config: self.config.clone(),
            messenger: messenger::from_config(self.config.clone()),
            github_session: self.github_session.clone(),
            pr_merge: self.pr_merge_worker.new_sender(),
        };

        match handler.handle_event() {
            Some(r) => Ok(r),
            None => Ok(Response::with((status::Ok, format!("Unhandled event: {}", event)))),
        }
    }
}

impl GithubEventHandler {
    fn handle_event(&self) -> Option<Response> {
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
        self.config.users.slack_user_name(user.login(), &self.data.repository)
    }

    fn handle_ping(&self) -> Response {
        Response::with((status::Ok, "ping"))
    }

    fn handle_pr(&self) -> Response {
        if let Some(ref pull_request) = self.data.pull_request {
            let verb: Option<String>;
            if self.action == "opened" {
                verb = Some(format!("opened by {}", self.slack_user_name(&pull_request.user)));
            } else if self.action == "closed" {
                if pull_request.merged == Some(true) {
                    verb = Some("merged".to_string());
                } else {
                    verb = Some("closed".to_string());
                }
            } else if self.action == "reopened" {
                verb = Some("reopened".to_string());
            } else if self.action == "assigned" {
                let assignees_str = self.config
                    .users
                    .slack_user_names(&pull_request.assignees, &self.data.repository)
                    .join(", ");
                verb = Some(format!("assigned to {}", assignees_str));
            } else if self.action == "unassigned" {
                verb = Some("unassigned".to_string());
            } else {
                verb = None;
            }

            if let Some(ref verb) = verb {
                let msg = format!("Pull Request {}", verb);
                let attachments = vec![SlackAttachmentBuilder::new("")
                                           .title(format!("Pull Request #{}: \"{}\"",
                                                          pull_request.number,
                                                          pull_request.title.as_str()))
                                           .title_link(pull_request.html_url.as_str())
                                           .build()];

                self.messenger.send_to_all(&msg,
                                           &attachments,
                                           &pull_request.user,
                                           &self.data.sender,
                                           &self.data.repository,
                                           &pull_request.assignees);
            }

            if self.action == "labeled" {
                if let Some(ref label) = self.data.label {
                    self.merge_pull_request(pull_request, label);
                }
            } else if verb == Some("merged".to_string()) {
                self.merge_pull_request_all_labels(pull_request);
            }
        }

        Response::with((status::Ok, "pr"))
    }

    fn handle_pr_review_comment(&self) -> Response {
        if let Some(ref pull_request) = self.data.pull_request {
            if let Some(ref comment) = self.data.comment {
                if self.action == "created" {
                    self.do_pull_request_comment(pull_request,
                                                 &comment.user,
                                                 comment.body.as_str(),
                                                 comment.html_url.as_str());
                }

            }
        }

        Response::with((status::Ok, "pr_review_comment"))
    }

    fn handle_pr_review(&self) -> Response {
        if let Some(ref pull_request) = self.data.pull_request {
            if let Some(ref review) = self.data.review {
                if self.action == "submitted" {

                    // just a comment. should just be handled by regular comment handler.
                    if review.state == "commented" {
                        self.do_pull_request_comment(pull_request,
                                                     &review.user,
                                                     review.body.as_str(),
                                                     review.html_url.as_str());
                        return Response::with((status::Ok, "pr_review [comment]"));
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
                        return Response::with((status::Ok, "pr_review [ignored]"));
                    }

                    let msg = format!("{} {} PR \"{}\"",
                                      self.slack_user_name(&review.user),
                                      action_msg,
                                      util::make_link(pull_request.html_url.as_str(),
                                                      pull_request.title.as_str()));

                    let attachments = vec![SlackAttachmentBuilder::new(review.body.as_str())
                                               .title(format!("Review: {}", state_msg))
                                               .title_link(review.html_url.as_str())
                                               .color(color)
                                               .build()];

                    self.messenger.send_to_all(&msg,
                                               &attachments,
                                               &pull_request.user,
                                               &self.data.sender,
                                               &self.data.repository,
                                               &pull_request.assignees);

                }
            }
        }

        Response::with((status::Ok, "pr_review"))
    }

    fn do_pull_request_comment(&self, pull_request: &github::PullRequest,
                               commenter: &github::User, comment_body: &str, comment_url: &str) {
        if comment_body.trim().len() == 0 {
            return;
        }

        let msg = format!("Comment on \"{}\"",
                          util::make_link(pull_request.html_url.as_str(),
                                          pull_request.title.as_str()));

        let attachments = vec![SlackAttachmentBuilder::new(comment_body)
                                   .title(format!("{} said:", self.slack_user_name(&commenter)))
                                   .title_link(comment_url)
                                   .build()];

        self.messenger.send_to_all(&msg,
                                   &attachments,
                                   &pull_request.user,
                                   &self.data.sender,
                                   &self.data.repository,
                                   &pull_request.assignees);

    }

    fn handle_commit_comment(&self) -> Response {
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

                    let attachments = vec![SlackAttachmentBuilder::new(comment.body.as_str())
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

        Response::with((status::Ok, "commit_comment"))
    }

    fn handle_issue_comment(&self) -> Response {
        if let Some(ref issue) = self.data.issue {
            if let Some(ref comment) = self.data.comment {
                if self.action == "created" {
                    let msg = format!("Comment on \"{}\"",
                                      util::make_link(issue.html_url.as_str(),
                                                      issue.title.as_str()));

                    let attachments = vec![SlackAttachmentBuilder::new(comment.body.as_str())
                                               .title(format!("{} said:",
                                                              self.slack_user_name(&comment.user)))
                                               .title_link(comment.html_url.as_str())
                                               .build()];

                    self.messenger.send_to_all(&msg,
                                               &attachments,
                                               &issue.user,
                                               &self.data.sender,
                                               &self.data.repository,
                                               &issue.assignees);
                }
            }
        }
        Response::with((status::Ok, "issue_comment"))
    }

    fn handle_push(&self) -> Response {
        Response::with((status::Ok, "push"))
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
        let mut attachment = SlackAttachmentBuilder::new("");

        attachment.title(format!("Source PR: #{}: \"{}\"",
                           pull_request.number,
                           pull_request.title)
                .as_str())
            .title_link(pull_request.html_url.clone());

        if let Err(e) = self.pr_merge
            .send(PRMergeMessage::merge(&self.data.repository, pull_request, &target_branch)) {
            error!("Error sending merge request message: {}", e)
        }
    }
}
