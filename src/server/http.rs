use super::super::std;
use super::*;

use std::io::Read;
use std::fmt;
use super::iron::prelude::*;
use super::iron::{status, BeforeMiddleware};
use super::router::Router;
use super::super::ring::{digest, hmac};

pub fn start(config: Config) -> Result<(), String> {
    let mut router = Router::new();
    router.post("/", webhook_handler, "webhook");

    let default_listen = String::from("0.0.0.0:3000");
    let addr_and_port = match config.listen_addr {
        Some(ref addr_and_port) => addr_and_port,
        None => &default_listen,
    };

    let mut chain = Chain::new(router);
    chain.link_before(GithubWebhookVerifier { secret: config.github_secret.clone() });

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
    fn is_valid(&self, body: &Vec<u8>, signature: &Vec<u8>) -> bool {
        let key = hmac::VerificationKey::new(&digest::SHA1, self.secret.as_ref());
        match hmac::verify(&key, &body.as_ref(), &signature.as_ref()) {
            Ok(_) => true,
            Err(_) => false,
        }
    }
}

impl BeforeMiddleware for GithubWebhookVerifier {
    fn before(&self, req: &mut Request) -> IronResult<()> {
        let sig_header: &Vec<u8> = match req.headers.get_raw("x-hub-signature") {
            Some(h) => {
                if h.len() == 0 || h.len() > 1 {
                    return Err(IronError::new(StringError("Expected to find exactly one signature header".to_string()), status::BadRequest))
                } else {
                    &h[0]
                }
            },
            None => return Err(IronError::new(StringError("Expected to find exactly one signature header".to_string()), status::BadRequest)),
        };

        // assume it starts with 'sha1='
        if sig_header.len() < 6 {
            return Err(IronError::new(StringError("Invalid signature value".to_string()), status::BadRequest));
        }
        if String::from_utf8_lossy(&sig_header[0..4]) != "sha1=" {
            return Err(IronError::new(StringError("Invalid signature value. Expected SHA1".to_string()), status::BadRequest));
        }
        let sig_header = &sig_header[5..].to_vec();

        let mut body_raw: Vec<u8> = vec![];
        match req.body.read_to_end(&mut body_raw) {
            Ok(_) => (),
            Err(e) =>  return Err(IronError::new(StringError(format!("Error reading body: {}", e)), status::InternalServerError)),
        };

        if self.is_valid(&body_raw, sig_header)  {
            Ok(())
        } else {
            Err(IronError::new(StringError("Invalid signature!".to_string()), status::BadRequest))
        }
    }
}


fn webhook_handler(_: &mut Request) -> IronResult<Response> {
    Ok(Response::with((status::Ok, "Hello, Octobot!")))
}
