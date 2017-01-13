use std::sync::Arc;

use super::iron::prelude::*;
use super::iron::status;
use super::iron::middleware::Handler;
use super::bodyparser;
use super::super::rustc_serialize::json;
use super::super::url::Url;

use super::super::git::Git;
use super::super::github;
use super::super::slack::SlackAttachmentBuilder;
use super::super::util;
use super::super::messenger::Messenger;
use super::super::users::UserConfig;
use super::super::repos::RepoConfig;


pub struct GithubHandler {
    pub git: Arc<Git>,
    pub messenger: Arc<Messenger>,
    pub users: Arc<UserConfig>,
    pub repos: Arc<RepoConfig>,
}

impl Handler for GithubHandler {
    fn handle(&self, req: &mut Request) -> IronResult<Response> {
        let event: String = match req.headers.get_raw("x-github-event") {
            Some(ref h) if h.len() == 1 => String::from_utf8_lossy(&h[0]).into_owned(),
            None | Some(..) => {
                return Ok(Response::with((status::BadRequest,
                                          "Expected to find exactly one event header")))
            }
        };

        let body = match req.get::<bodyparser::Raw>() {
            Ok(Some(j)) => j,
            Err(_) | Ok(None) => {
                return Ok(Response::with((status::BadRequest, format!("Error reading json"))))
            }
        };

        let data: github::HookBody = match json::decode(body.as_str()) {
            Ok(h) => h,
            Err(e) => {
                return Ok(Response::with((status::BadRequest,
                                          format!("Error parsing JSON: {}", e))))
            }
        };

        match self.handle_event(&event, &data) {
            Some(r) => Ok(r),
            None => Ok(Response::with((status::Ok, format!("Unhandled event: {}", event)))),
        }
    }
}

impl GithubHandler {
    fn handle_event(&self, event: &String, data: &github::HookBody) -> Option<Response> {
        info!("Received event: {}", event);
        if event == "ping" {
            Some(self.handle_ping(data))
        } else if event == "pull_request" {
            Some(self.handle_pr(data))
        } else if event == "pull_request_review_comment" {
            Some(self.handle_pr_review_comment(data))
        } else if event == "pull_request_review" {
            Some(self.handle_pr_review(data))
        } else if event == "commit_comment" {
            Some(self.handle_commit_comment(data))
        } else if event == "issue_comment" {
            Some(self.handle_issue_comment(data))
        } else if event == "push" {
            Some(self.handle_push(data))
        } else {
            None
        }
    }

    fn slack_user_name(&self, user: &github::User, data: &github::HookBody) -> String {
        self.users.slack_user_name(user.login.as_str(), &data.repository)
    }

    fn handle_ping(&self, data: &github::HookBody) -> Response {
        Response::with((status::Ok, "ping"))
    }

    fn handle_pr(&self, data: &github::HookBody) -> Response {
        if let Some(ref pull_request) = data.pull_request {
            let verb: Option<String>;
            if data.action == Some("opened".to_string()) {
                verb = Some(format!("opened by {}",
                                    self.slack_user_name(&pull_request.user, data)));
            } else if data.action == Some("closed".to_string()) {
                if pull_request.merged == Some(true) {
                    verb = Some("merged".to_string());
                } else {
                    verb = Some("closed".to_string());
                }
            } else if data.action == Some("reopened".to_string()) {
                verb = Some("reopened".to_string());
            } else if data.action == Some("assigned".to_string()) {
                let assignees_str = self.users
                    .slack_user_names(&pull_request.assignees, &data.repository)
                    .join(", ");
                verb = Some(format!("assigned to {}", assignees_str));
            } else if data.action == Some("unassigned".to_string()) {
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
                                           &data.sender,
                                           &data.repository,
                                           &pull_request.assignees);
            }

            if data.action == Some("labeled".to_string()) {
                // mergePullRequest(messenger, githubAPI, data.pull_request, data.repository, data.label);
            } else if verb == Some("merged".to_string()) {
                // mergePullRequestAllLabels(messenger, githubAPI, data.pull_request, data.repository);
            }
        }

        Response::with((status::Ok, "pr"))
    }

    fn handle_pr_review_comment(&self, data: &github::HookBody) -> Response {
        if let Some(ref pull_request) = data.pull_request {
            if let Some(ref comment) = data.comment {
                if data.action == Some("created".to_string()) {
                    self.do_pull_request_comment(pull_request,
                                                 &comment.user,
                                                 comment.body.as_str(),
                                                 comment.html_url.as_str(),
                                                 data);
                }

            }
        }

        Response::with((status::Ok, "pr_review_comment"))
    }

    fn handle_pr_review(&self, data: &github::HookBody) -> Response {
        if let Some(ref pull_request) = data.pull_request {
            if let Some(ref review) = data.review {
                if data.action == Some("submitted".to_string()) {

                    // just a comment. should just be handled by regular comment handler.
                    if review.state == "commented" {
                        self.do_pull_request_comment(pull_request,
                                                     &review.user,
                                                     review.body.as_str(),
                                                     review.html_url.as_str(),
                                                     data);
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
                                      self.slack_user_name(&review.user, data),
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
                                               &data.sender,
                                               &data.repository,
                                               &pull_request.assignees);

                }
            }
        }

        Response::with((status::Ok, "pr_review"))
    }

    fn do_pull_request_comment(&self,
                               pull_request: &github::PullRequest,
                               commenter: &github::User,
                               comment_body: &str,
                               comment_url: &str,
                               data: &github::HookBody) {
        if comment_body.trim().len() == 0 {
            return;
        }

        let msg = format!("Comment on \"{}\"",
                          util::make_link(pull_request.html_url.as_str(),
                                          pull_request.title.as_str()));

        let attachments = vec![SlackAttachmentBuilder::new(comment_body)
                                   .title(format!("{} said:",
                                                  self.slack_user_name(&commenter, data)))
                                   .title_link(comment_url)
                                   .build()];

        self.messenger.send_to_all(&msg,
                                   &attachments,
                                   &pull_request.user,
                                   &data.sender,
                                   &data.repository,
                                   &pull_request.assignees);

    }

    fn handle_commit_comment(&self, data: &github::HookBody) -> Response {
        if let Some(ref comment) = data.comment {
            if data.action == Some("created".to_string()) {
                let commit: &str = &comment.commit_id[0..7];
                let commit_url =
                    format!("{}/commit/{}", data.repository.html_url, comment.commit_id);
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
                                                          self.slack_user_name(&comment.user,
                                                                               data)))
                                           .title_link(comment.html_url.as_str())
                                           .build()];

                self.messenger.send_to_all(&msg,
                                           &attachments,
                                           &comment.user,
                                           &data.sender,
                                           &data.repository,
                                           &vec![]);
            }
        }

        Response::with((status::Ok, "commit_comment"))
    }

    fn handle_issue_comment(&self, data: &github::HookBody) -> Response {
        if let Some(ref issue) = data.issue {
            if let Some(ref comment) = data.comment {
                if data.action == Some("created".to_string()) {
                    let msg = format!("Comment on \"{}\"",
                                      util::make_link(issue.html_url.as_str(),
                                                      issue.title.as_str()));

                    let attachments = vec![SlackAttachmentBuilder::new(comment.body.as_str())
                                               .title(format!("{} said:",
                                                              self.slack_user_name(&comment.user,
                                                                                   data)))
                                               .title_link(comment.html_url.as_str())
                                               .build()];

                    self.messenger.send_to_all(&msg,
                                               &attachments,
                                               &issue.user,
                                               &data.sender,
                                               &data.repository,
                                               &issue.assignees);
                }
            }
        }
        Response::with((status::Ok, "issue_comment"))
    }

    fn handle_push(&self, data: &github::HookBody) -> Response {
        Response::with((status::Ok, "push"))
    }
}
