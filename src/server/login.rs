use std::sync::Arc;

use hyper::header::ContentType;
use hyper::server::{Request, Response};
use hyper::StatusCode;
use ring::{digest, pbkdf2};
use rustc_serialize::hex::{ToHex, FromHex};

use config::Config;
use server::sessions::Sessions;
use server::http::{FutureResponse, OctobotHandler, OctobotFilter, OctobotFilterResult, parse_json};

static DIGEST_ALG: &'static digest::Algorithm = &digest::SHA256;
const CREDENTIAL_LEN: usize = digest::SHA256_OUTPUT_LEN;
const PBKDF2_ITERATIONS: u32 = 20_000;

pub fn store_password(pass: &str, salt: &str) -> String {
    let mut pass_hash = [0u8; CREDENTIAL_LEN];
    pbkdf2::derive(DIGEST_ALG, PBKDF2_ITERATIONS, salt.as_bytes(),
                   pass.as_bytes(), &mut pass_hash);

    pass_hash.to_hex()
}

pub fn verify_password(pass: &str, salt: &str, pass_hash: &str) -> bool {
    let pass_hash = match pass_hash.from_hex() {
        Ok(h) => h,
        Err(e) => {
            error!("Invalid password hash stored: {} -- {}", pass_hash, e);
            return false
        }
    };
    pbkdf2::verify(DIGEST_ALG, PBKDF2_ITERATIONS, salt.as_bytes(),
                   pass.as_bytes(),
                   &pass_hash).is_ok()
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
    pub fn new(sessions: Arc<Sessions>, config: Arc<Config>) -> Box<LoginHandler> {
        Box::new(LoginHandler {
            sessions: sessions,
            config: config,
        })
    }
}

impl LogoutHandler {
    pub fn new(sessions: Arc<Sessions>) -> Box<LogoutHandler> {
        Box::new(LogoutHandler {
            sessions: sessions,
        })
    }
}

impl LoginSessionFilter {
    pub fn new(sessions: Arc<Sessions>) -> Box<LoginSessionFilter> {
        Box::new(LoginSessionFilter {
            sessions: sessions,
        })
    }
}

#[derive(Deserialize, Clone)]
struct LoginRequest {
    username: String,
    password: String,
}

fn get_session(req: &Request) -> Option<String> {
    match req.headers().get_raw("session") {
        Some(ref h) if h.len() == 1 => Some(String::from_utf8_lossy(&h[0]).into_owned()),
        None | Some(..) => None,
    }
}

impl OctobotHandler for LoginHandler {
    fn handle(&self, req: Request) -> FutureResponse {
        let admin = self.config.admin.clone();
        let sessions = self.sessions.clone();

        parse_json(req, move |login_req: LoginRequest| {
            let success = match admin {
                None => false,
                Some(ref admin) => {
                    let pw_correct = verify_password(&login_req.password, &admin.salt, &admin.pass_hash);
                    admin.name == login_req.username && pw_correct
                }
            };

            if success {
                let sess_id = sessions.new_session();
                let json = json!({
                    "session": sess_id,
                });

                Response::new()
                    .with_header(ContentType::json())
                    .with_body(json.to_string())
            } else {
                Response::new().with_status(StatusCode::Unauthorized)
            }
        })
    }
}


fn invalid_session() -> Response {
    Response::new()
        .with_status(StatusCode::Forbidden)
        .with_body("Invalid session")
}

impl OctobotHandler for LogoutHandler {
    fn handle(&self, req: Request) -> FutureResponse {
        let sess: String = match get_session(&req) {
            Some(s) => s.to_string(),
            None => {
                return self.respond(invalid_session())
            }
        };

        self.sessions.remove_session(&sess);
        self.respond(
            Response::new()
                .with_header(ContentType::json())
                .with_body("{}")
        )
    }
}

impl OctobotFilter for LoginSessionFilter {
    fn filter(&self, req: &Request) -> OctobotFilterResult {
        let sess: String = match get_session(&req) {
            Some(s) => s.to_string(),
            None => {
                return OctobotFilterResult::Halt(invalid_session())
            }
        };

        if self.sessions.is_valid_session(&sess) {
            OctobotFilterResult::Continue
        } else {
            OctobotFilterResult::Halt(invalid_session())
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
