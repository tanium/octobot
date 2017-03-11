use std::sync::Arc;

use bodyparser;
use iron::prelude::*;
use iron::status;
use iron::headers::ContentType;
use iron::middleware::{BeforeMiddleware, Handler};
use iron::modifiers::Header;
use serde_json;
use ring::digest;
use rustc_serialize::hex::ToHex;

use config::Config;
use server::github_verify::StringError;
use server::sessions::Sessions;

pub fn hash_password(pass: &str, salt: &str) -> String {
    let mut ctx = digest::Context::new(&digest::SHA256);
    ctx.update(salt.as_bytes());
    ctx.update(pass.as_bytes());

    ctx.finish().as_ref().to_hex()
}

pub struct LoginHandler {
    sessions: Arc<Sessions>,
    config: Arc<Config>,
}

pub struct LogoutHandler {
    sessions: Arc<Sessions>,
}

pub struct LoginSessionFilter {
    sessions: Arc<Sessions>,
}

impl LoginHandler {
    pub fn new(sessions: Arc<Sessions>, config: Arc<Config>) -> LoginHandler {
        LoginHandler {
            sessions: sessions,
            config: config,
        }
    }
}

impl LogoutHandler {
    pub fn new(sessions: Arc<Sessions>) -> LogoutHandler {
        LogoutHandler {
            sessions: sessions,
        }
    }
}

impl LoginSessionFilter {
    pub fn new(sessions: Arc<Sessions>) -> LoginSessionFilter {
        LoginSessionFilter {
            sessions: sessions,
        }
    }
}

#[derive(Deserialize)]
struct LoginRequest {
    username: String,
    password: String,
}

fn get_session(req: &Request) -> IronResult<String> {
    match req.headers.get_raw("session") {
        Some(ref h) if h.len() == 1 => Ok(String::from_utf8_lossy(&h[0]).into_owned()),
        None | Some(..) => {
            return Err(IronError::new(StringError::new("No session header found"),
                                      status::Forbidden))
        }
    }
}

impl Handler for LoginHandler {
    fn handle(&self, req: &mut Request) -> IronResult<Response> {
        let body = match req.get::<bodyparser::Raw>() {
            Ok(Some(j)) => j,
            Err(_) | Ok(None) => {
                error!("Error reading json");
                return Ok(Response::with((status::BadRequest, format!("Error reading json"))));
            }
        };

        let login_req: LoginRequest = match serde_json::from_str(&body) {
            Ok(r) => r,
            Err(e) => {
                error!("Error parsing login request: {}", e);
                return Ok(Response::with((status::BadRequest,
                                          format!("Error parsing JSON: {}", e))));
            }
        };


        let success = match self.config.admin {
            None => false,
            Some(ref admin) => {
                let hash = hash_password(&login_req.password, &admin.salt);
                admin.name == login_req.username && hash == admin.pass_hash
            }
        };

        if success {
            let sess = self.sessions.new_session();
            let json = format!("{{\"session\": \"{}\"}}", sess);
            Ok(Response::with((status::Ok, Header(ContentType::json()), json)))

        } else {
            Ok(Response::with(status::Unauthorized))
        }

    }
}

impl Handler for LogoutHandler {
    fn handle(&self, req: &mut Request) -> IronResult<Response> {
        let sess = try!(get_session(req));
        self.sessions.remove_session(&sess);
        Ok(Response::with((status::Ok, Header(ContentType::json()), "{}")))
    }
}

impl BeforeMiddleware for LoginSessionFilter {
    fn before(&self, req: &mut Request) -> IronResult<()> {
        let sess = try!(get_session(req));
        if self.sessions.is_valid_session(&sess) {
            Ok(())
        } else {
            Err(IronError::new(StringError::new("Invalid session"), status::Forbidden))
        }
    }
}
