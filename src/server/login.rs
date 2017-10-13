use std::sync::Arc;

use bodyparser;
use iron::prelude::*;
use iron::status;
use iron::headers::ContentType;
use iron::middleware::{BeforeMiddleware, Handler};
use iron::modifiers::Header;
use ring::{digest, pbkdf2};
use rustc_serialize::hex::{ToHex, FromHex};

use config::Config;
use server::github_verify::StringError;
use server::sessions::Sessions;

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

#[derive(Deserialize, Clone)]
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
        let login_req = match req.get::<bodyparser::Struct<LoginRequest>>() {
            Ok(Some(r)) => r,
            Err(_) | Ok(None) => {
                error!("Error reading json");
                return Ok(Response::with((status::BadRequest, format!("Error reading json"))));
            }
        };

        let success = match self.config.admin {
            None => false,
            Some(ref admin) => {
                let pw_correct = verify_password(&login_req.password, &admin.salt, &admin.pass_hash);
                admin.name == login_req.username && pw_correct
            }
        };

        if success {
            let sess_id = self.sessions.new_session();
            let json = json!({
                "session": sess_id,
            });
            Ok(Response::with((status::Ok, Header(ContentType::json()), json.to_string())))

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
