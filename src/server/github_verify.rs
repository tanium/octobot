use std;

use std::fmt;
use ring::{digest, hmac};
use iron::prelude::*;
use iron::{status, BeforeMiddleware};
use rustc_serialize::hex::FromHex;
use bodyparser;

#[derive(Debug)]
pub struct StringError(String);

impl StringError {
    pub fn new(val: &str) -> StringError {
        StringError(val.into())
    }
}

impl fmt::Display for StringError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

impl std::error::Error for StringError {
    fn description(&self) -> &str {
        &*self.0
    }
}

pub struct GithubWebhookVerifier {
    pub secret: String,
}

impl GithubWebhookVerifier {
    fn is_valid(&self, data: &[u8], signature: &String) -> IronResult<()> {
        // assume it starts with 'sha1='
        if signature.len() < 6 {
            return Err(IronError::new(StringError("Invalid signature value".to_string()),
                                      status::BadRequest));
        }
        let sig_prefix = &signature[0..5];
        if sig_prefix != "sha1=" {
            return Err(IronError::new(StringError(format!("Invalid signature value. Expected \
                                                           'sha1='; found: '{}'",
                                                          sig_prefix)),
                                      status::BadRequest));
        }

        let sig_bytes: Vec<u8> = match signature[5..].from_hex() {
            Ok(s) => s,
            Err(e) => {
                return Err(IronError::new(StringError(format!("Invalid hex value: {}", e)),
                                          status::BadRequest))
            }
        };

        let key = hmac::VerificationKey::new(&digest::SHA1, self.secret.as_bytes());
        match hmac::verify(&key, data, &sig_bytes) {
            Ok(_) => Ok(()),
            Err(_) => {
                Err(IronError::new(StringError("Invalid signature!".to_string()),
                                   status::BadRequest))
            }
        }
    }
}

impl BeforeMiddleware for GithubWebhookVerifier {
    fn before(&self, req: &mut Request) -> IronResult<()> {
        let sig_header: String = match req.headers.get_raw("x-hub-signature") {
            Some(ref h) if h.len() == 1 => String::from_utf8_lossy(&h[0]).into_owned(),
            None | Some(..) => {
                return Err(IronError::new(StringError("Expected to find exactly one signature \
                                                       header"
                                              .to_string()),
                                          status::BadRequest))
            }
        };

        let body = match req.get::<bodyparser::Raw>() {
            Ok(Some(body)) => body,
            Err(_) | Ok(None) => {
                return Err(IronError::new(StringError("Error reading body".to_string()),
                                          status::InternalServerError))
            }
        };

        self.is_valid(body.as_bytes(), &sig_header)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::{digest, hmac};
    use rustc_serialize::hex::ToHex;

    #[test]
    fn verify_sig_valid() {
        let key_value = String::from("this is my secret key!");
        let key = hmac::SigningKey::new(&digest::SHA1, key_value.as_bytes());

        let msg = "a message from the githubs.";
        let signature = hmac::sign(&key, msg.as_bytes());
        let signature_hex = "sha1=".to_string() + signature.as_ref().to_hex().as_str();

        let verifier = GithubWebhookVerifier { secret: key_value.clone() };

        assert!(verifier.is_valid(msg.as_bytes(), &signature_hex).is_ok());
    }

    #[test]
    fn verify_sig_wrong_digest() {
        let key_value = String::from("this is my secret key!");
        let key = hmac::SigningKey::new(&digest::SHA1, key_value.as_bytes());

        let msg = "a message from the githubs.";
        let signature = hmac::sign(&key, msg.as_bytes());
        let signature_hex = "sha9=".to_string() + signature.as_ref().to_hex().as_str();

        let verifier = GithubWebhookVerifier { secret: key_value.clone() };

        assert!(verifier.is_valid(msg.as_bytes(), &signature_hex).is_err());
    }

    #[test]
    fn verify_sig_missing_header() {
        let key_value = String::from("this is my secret key!");
        let key = hmac::SigningKey::new(&digest::SHA1, key_value.as_bytes());

        let msg = "a message from the githubs.";
        let signature = hmac::sign(&key, msg.as_bytes());
        let signature_hex = signature.as_ref().to_hex();

        let verifier = GithubWebhookVerifier { secret: key_value.clone() };

        assert!(verifier.is_valid(msg.as_bytes(), &signature_hex).is_err());
    }
}
