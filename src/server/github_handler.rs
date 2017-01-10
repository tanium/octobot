use std::sync::Arc;

use super::iron::prelude::*;
use super::iron::status;
use super::iron::middleware::Handler;
use super::bodyparser;
use super::super::serde_json::Value;

use super::super::git::Git;
use super::super::messenger::Messenger;

pub struct GithubHandler {
    pub git: Arc<Git>,
    pub messenger: Arc<Messenger>,
}

impl Handler for GithubHandler {
    fn handle(&self, req: &mut Request) -> IronResult<Response> {
        let json_body = match req.get::<bodyparser::Json>() {
            Ok(Some(j)) => j,
            Err(_) | Ok(None) => {
                return Ok(Response::with((status::BadRequest, format!("Error parsing json"))))
            }
        };

        let event: String = match req.headers.get_raw("x-github-event") {
            Some(ref h) if h.len() == 1 => String::from_utf8_lossy(&h[0]).into_owned(),
            None | Some(..) => {
                return Ok(Response::with((status::BadRequest,
                                          "Expected to find exactly one event header")))
            }
        };

        match self.handle_event(&event, &json_body) {
            Some(r) => Ok(r),
            None => Ok(Response::with((status::Ok, format!("Unhandled event: {}", event))))
        }
    }
}

impl GithubHandler {
    fn handle_event(&self, event: &String, body: &Value) -> Option<Response> {
        info!("Received event: {}", event);
        if event == "ping" {
            Some(self.handle_ping(body))
        } else if event == "pull_request" {
            Some(self.handle_pr(body))
        } else if event == "pull_request_review_comment" {
            Some(self.handle_pr_review_comment(body))
        } else if event == "pull_request_review" {
            Some(self.handle_pr_review(body))
        } else if event == "commit_comment" {
            Some(self.handle_commit_comment(body))
        } else if event == "issue_comment" {
            Some(self.handle_issue_comment(body))
        } else if event == "push" {
            Some(self.handle_push(body))
        } else {
            None
        }
    }

    fn handle_ping(&self, body: &Value) -> Response {
        Response::with((status::Ok, "ping"))
    }

    fn handle_pr(&self, body: &Value) -> Response {
        Response::with((status::Ok, "pr"))
    }

    fn handle_pr_review_comment(&self, body: &Value) -> Response {
        Response::with((status::Ok, "pr_review_comment"))
    }

    fn handle_pr_review(&self, body: &Value) -> Response {
        Response::with((status::Ok, "pr_review"))
    }

    fn handle_commit_comment(&self, body: &Value) -> Response {
        Response::with((status::Ok, "commit_comment"))
    }

    fn handle_issue_comment(&self, body: &Value) -> Response {
        Response::with((status::Ok, "issue_comment"))
    }

    fn handle_push(&self, body: &Value) -> Response {
        Response::with((status::Ok, "push"))
    }

}
