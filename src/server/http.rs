use super::super::std;
use super::*;

use std::io::Read;
use std::fmt;
use super::iron::prelude::*;
use super::iron::{status, BeforeMiddleware};
use super::router::Router;
use super::super::ring::{digest, hmac};
use super::super::logger::Logger;
use super::super::rustc_serialize::hex::FromHex;

pub fn start(config: Config) -> Result<(), String> {
    let mut router = Router::new();
    router.post("/", webhook_handler, "webhook");

    let default_listen = String::from("0.0.0.0:3000");
    let addr_and_port = match config.listen_addr {
        Some(ref addr_and_port) => addr_and_port,
        None => &default_listen,
    };

    let mut chain = Chain::new(router);
    let (logger_before, logger_after) = Logger::new(None);

    // before first middleware
    chain.link_before(logger_before);

    chain.link_before(GithubWebhookVerifier { secret: config.github_secret.clone() });

    // after last middleware
    chain.link_after(logger_after);

    match Iron::new(chain).http(addr_and_port.as_str()) {
        Ok(_) => {
            println!("Listening on port {}", addr_and_port);
            Ok(())
        },
        Err(e) => Err(format!("{}", e)),
    }
}

#[derive(Debug)]
struct StringError(String);

impl fmt::Display for StringError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

impl std::error::Error for StringError {
    fn description(&self) -> &str { &*self.0 }
}
struct GithubWebhookVerifier {
    secret: String,
}

impl GithubWebhookVerifier {
    fn is_valid(&self, body: &Vec<u8>, signature: String) -> IronResult<()> {
        // assume it starts with 'sha1='
        if signature.len() < 6 {
            return Err(IronError::new(StringError("Invalid signature value".to_string()), status::BadRequest));
        }
        let sig_prefix = &signature[0..5];
        if sig_prefix != "sha1=" {
            return Err(IronError::new(StringError(format!("Invalid signature value. Expected 'sha1='; found: '{}'", sig_prefix)), status::BadRequest));
        }
        println!("SIG0={}", signature);
        println!("SIG1={}", &signature[5..]);

        let sig_bytes: Vec<u8> = match signature[5..].from_hex() {
            Ok(s) => s,
            Err(e) => return Err(IronError::new(StringError(format!("Invalid hex value: {}", e)), status::BadRequest)),
        };

        let key = hmac::VerificationKey::new(&digest::SHA1, self.secret.as_bytes());
        match hmac::verify(&key, &body.as_ref(), &sig_bytes) {
            Ok(_) => Ok(()),
            Err(_) => Err(IronError::new(StringError("Invalid signature!".to_string()), status::BadRequest))
        }
    }
}

impl BeforeMiddleware for GithubWebhookVerifier {
    fn before(&self, req: &mut Request) -> IronResult<()> {
        let sig_header: String = match req.headers.get_raw("x-hub-signature") {
            Some(h) => {
                if h.len() == 0 || h.len() > 1 {
                    return Err(IronError::new(StringError("Expected to find exactly one signature header".to_string()), status::BadRequest))
                } else {
                    String::from_utf8_lossy(&h[0]).into_owned()
                }
            },
            None => return Err(IronError::new(StringError("Expected to find exactly one signature header".to_string()), status::BadRequest)),
        };


        let mut body_raw: Vec<u8> = vec![];
        match req.body.read_to_end(&mut body_raw) {
            Ok(_) => (),
            Err(e) =>  return Err(IronError::new(StringError(format!("Error reading body: {}", e)), status::InternalServerError)),
        };

        self.is_valid(&body_raw, sig_header)
    }
}


fn webhook_handler(_: &mut Request) -> IronResult<Response> {
    Ok(Response::with((status::Ok, "Hello, Octobot!")))
}
