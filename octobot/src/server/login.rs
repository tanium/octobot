use std::sync::Arc;

use async_trait::async_trait;
use hyper::{Body, Request, Response, StatusCode};
use log::{error, info, warn};
use serde_derive::Deserialize;
use serde_json::json;

use octobot_lib::config::Config;
use octobot_lib::errors::*;
use octobot_lib::passwd;

use crate::http_util;
use crate::server::http::{parse_json, Filter, FilterResult, Handler};
use crate::server::sessions::Sessions;

pub struct LoginHandler {
    sessions: Arc<Sessions>,
    config: Arc<Config>,
}

pub struct LogoutHandler {
    sessions: Arc<Sessions>,
}

pub struct SessionCheckHandler {
    sessions: Arc<Sessions>,
}

pub struct LoginSessionFilter {
    sessions: Arc<Sessions>,
}

impl LoginHandler {
    pub fn new(sessions: Arc<Sessions>, config: Arc<Config>) -> Box<LoginHandler> {
        Box::new(LoginHandler {
            sessions: sessions,
            config: config,
        })
    }
}

impl LogoutHandler {
    pub fn new(sessions: Arc<Sessions>) -> Box<LogoutHandler> {
        Box::new(LogoutHandler { sessions: sessions })
    }
}

impl SessionCheckHandler {
    pub fn new(sessions: Arc<Sessions>) -> Box<SessionCheckHandler> {
        Box::new(SessionCheckHandler { sessions: sessions })
    }
}

impl LoginSessionFilter {
    pub fn new(sessions: Arc<Sessions>) -> Box<LoginSessionFilter> {
        Box::new(LoginSessionFilter { sessions: sessions })
    }
}

#[derive(Deserialize, Clone)]
struct LoginRequest {
    username: String,
    password: String,
}

fn get_session(req: &Request<Body>) -> Option<String> {
    req.headers()
        .get("session")
        .map(|h| String::from_utf8_lossy(h.as_bytes()).into_owned())
}

#[async_trait]
impl Handler for LoginHandler {
    async fn handle(&self, req: Request<Body>) -> Result<Response<Body>> {
        let config = self.config.clone();
        let sessions = self.sessions.clone();

        let login_req: LoginRequest = parse_json(req).await?;

        let mut success = None;
        if let Some(ref admin) = config.admin {
            if admin.name == login_req.username {
                if passwd::verify_password(&login_req.password, &admin.salt, &admin.pass_hash) {
                    info!("Admin auth success");
                    success = Some(true);
                } else {
                    warn!("Admin auth failure");
                    success = Some(false);
                }
            }
        }

        if success.is_none() {
            if let Some(ref ldap) = config.ldap {
                match octobot_ldap::auth(&login_req.username, &login_req.password, ldap) {
                    Ok(true) => {
                        info!("LDAP auth successfor user: {}", login_req.username);
                        success = Some(true)
                    }
                    Ok(false) => warn!("LDAP auth failure for user: {}", login_req.username),
                    Err(e) => error!("Error authenticating to LDAP: {}", e),
                };
            }
        }

        if success == Some(true) {
            let sess_id = sessions.new_session();
            let json = json!({
                "session": sess_id,
            });

            Ok(http_util::new_json_resp(json.to_string()))
        } else {
            Ok(http_util::new_empty_resp(StatusCode::UNAUTHORIZED))
        }
    }
}

fn invalid_session() -> Response<Body> {
    http_util::new_msg_resp(StatusCode::FORBIDDEN, "Invalid session")
}

#[async_trait]
impl Handler for LogoutHandler {
    async fn handle(&self, req: Request<Body>) -> Result<Response<Body>> {
        let sess: String = match get_session(&req) {
            Some(s) => s.to_string(),
            None => return Ok(invalid_session()),
        };

        self.sessions.remove_session(&sess);
        Ok(http_util::new_json_resp("{}".into()))
    }
}

#[async_trait]
impl Handler for SessionCheckHandler {
    async fn handle(&self, req: Request<Body>) -> Result<Response<Body>> {
        let sess: String = match get_session(&req) {
            Some(s) => s.to_string(),
            None => return Ok(invalid_session()),
        };

        if self.sessions.is_valid_session(&sess) {
            Ok(self.respond_with(StatusCode::OK, ""))
        } else {
            Ok(invalid_session())
        }
    }
}

impl Filter for LoginSessionFilter {
    fn filter(&self, req: Request<Body>) -> FilterResult {
        let sess: String = match get_session(&req) {
            Some(s) => s.to_string(),
            None => return FilterResult::Halt(invalid_session()),
        };

        if self.sessions.is_valid_session(&sess) {
            FilterResult::Continue(req)
        } else {
            FilterResult::Halt(invalid_session())
        }
    }
}
