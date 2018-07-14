use std::sync::Arc;

use hyper::{Body, Request, Response, StatusCode};
use ring::{digest, pbkdf2};
use rustc_serialize::hex::{FromHex, ToHex};

use config::Config;
use ldap_auth;
use server::http::{Filter, FilterResult, FutureResponse, Handler, parse_json};
use server::sessions::Sessions;
use util;

static DIGEST_ALG: &'static digest::Algorithm = &digest::SHA256;
const CREDENTIAL_LEN: usize = digest::SHA256_OUTPUT_LEN;
const PBKDF2_ITERATIONS: u32 = 20_000;

pub fn store_password(pass: &str, salt: &str) -> String {
    let mut pass_hash = [0u8; CREDENTIAL_LEN];
    pbkdf2::derive(DIGEST_ALG, PBKDF2_ITERATIONS, salt.as_bytes(), pass.as_bytes(), &mut pass_hash);

    pass_hash.to_hex()
}

pub fn verify_password(pass: &str, salt: &str, pass_hash: &str) -> bool {
    let pass_hash = match pass_hash.from_hex() {
        Ok(h) => h,
        Err(e) => {
            error!("Invalid password hash stored: {} -- {}", pass_hash, e);
            return false;
        }
    };
    pbkdf2::verify(DIGEST_ALG, PBKDF2_ITERATIONS, salt.as_bytes(), pass.as_bytes(), &pass_hash)
        .is_ok()
}

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
    req.headers().get("session").map(|h| String::from_utf8_lossy(h.as_bytes()).into_owned())
}

impl Handler for LoginHandler {
    fn handle(&self, req: Request<Body>) -> FutureResponse {
        let config = self.config.clone();
        let sessions = self.sessions.clone();

        parse_json(req, move |login_req: LoginRequest| {
            let mut success = None;
            if let Some(ref admin) = config.admin {
                if admin.name == login_req.username {
                    if verify_password(&login_req.password, &admin.salt, &admin.pass_hash) {
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
                    match ldap_auth::auth(&login_req.username, &login_req.password, ldap) {
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

                util::new_json_resp(json.to_string())
            } else {
                util::new_empty_resp(StatusCode::UNAUTHORIZED)
            }
        })
    }
}


fn invalid_session() -> Response<Body> {
    util::new_msg_resp(StatusCode::FORBIDDEN, "Invalid session")
}

impl Handler for LogoutHandler {
    fn handle(&self, req: Request<Body>) -> FutureResponse {
        let sess: String = match get_session(&req) {
            Some(s) => s.to_string(),
            None => return self.respond(invalid_session()),
        };

        self.sessions.remove_session(&sess);
        self.respond(util::new_json_resp("{}".into()))
    }
}

impl Handler for SessionCheckHandler {
    fn handle(&self, req: Request<Body>) -> FutureResponse {
        let sess: String = match get_session(&req) {
            Some(s) => s.to_string(),
            None => return self.respond(invalid_session()),
        };

        if self.sessions.is_valid_session(&sess) {
            self.respond_with(StatusCode::OK, "")
        } else {
            self.respond(invalid_session())
        }
    }
}

impl Filter for LoginSessionFilter {
    fn filter(&self, req: &Request<Body>) -> FilterResult {
        let sess: String = match get_session(&req) {
            Some(s) => s.to_string(),
            None => return FilterResult::Halt(invalid_session()),
        };

        if self.sessions.is_valid_session(&sess) {
            FilterResult::Continue
        } else {
            FilterResult::Halt(invalid_session())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_password() {
        let pw_hash = store_password("the-pass", "some-salt");
        assert_eq!(true, verify_password("the-pass", "some-salt", &pw_hash));
        assert_eq!(false, verify_password("wrong-pass", "some-salt", &pw_hash));
        assert_eq!(false, verify_password("the-pass", "wrong-salt", &pw_hash));
    }
}
