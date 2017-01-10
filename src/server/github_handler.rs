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

        self.handle_event(&event, &json_body)
    }
}

impl GithubHandler {
    fn handle_event(&self, event: &String, body: &Value) -> IronResult<Response> {
        Ok(Response::with((status::Ok, "Hello, Octobot!")))
    }
}
