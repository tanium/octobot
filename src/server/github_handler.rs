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

    fn handle_ping(&self, data: &github::HookBody) -> Response {
        Response::with((status::Ok, "ping"))
    }

    fn handle_pr(&self, data: &github::HookBody) -> Response {
        Response::with((status::Ok, "pr"))
    }

    fn handle_pr_review_comment(&self, data: &github::HookBody) -> Response {
        Response::with((status::Ok, "pr_review_comment"))
    }

    fn handle_pr_review(&self, data: &github::HookBody) -> Response {
        Response::with((status::Ok, "pr_review"))
    }

    fn handle_commit_comment(&self, data: &github::HookBody) -> Response {
        if let Some(ref comment) = data.comment {
            if let Some(ref action) = data.action {
                if action == "created" {
                    let commit: &str = &comment.commit_id[0..7];
                    let commit_url =
                        format!("{}/commit/{}", data.repository.html_url, comment.commit_id);
                    let commit_path: String;
                    if let Some(ref path) = comment.path {
                        commit_path = path.to_string();
                    } else {
                        commit_path = commit.to_string();
                    }

                    let msg = format!("Comment on \"{}\" ({})", commit_path, util::make_link(commit_url.as_str(), commit));

                    let slack_user = self.users
                        .slack_user_name(comment.user.login.as_str(), &data.repository);

                    let attach = SlackAttachmentBuilder::new(comment.body.as_str())
                        .title(format!("{} said:", slack_user))
                        .title_link(comment.html_url.as_str())
                        .build();

                    self.messenger.send_to_all(&msg,
                                               &vec![attach],
                                               &comment.user,
                                               &data.sender,
                                               &data.repository);
                }
            }
        }

        Response::with((status::Ok, "commit_comment"))
    }

    fn handle_issue_comment(&self, data: &github::HookBody) -> Response {
        Response::with((status::Ok, "issue_comment"))
    }

    fn handle_push(&self, data: &github::HookBody) -> Response {
        Response::with((status::Ok, "push"))
    }
}
