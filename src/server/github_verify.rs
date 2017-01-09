use super::super::std;

use std::fmt;
use std::io::Read;
use super::super::ring::{digest, hmac};
use super::iron::prelude::*;
use super::iron::{status, BeforeMiddleware};
use super::super::rustc_serialize::hex::FromHex;

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

pub struct GithubWebhookVerifier {
    pub secret: String,
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


