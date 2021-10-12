use hyper::HeaderMap;
use log::{debug, error};
use ring::hmac;
use rustc_serialize::hex::FromHex;

pub struct GithubWebhookVerifier {
    pub secret: String,
}

impl GithubWebhookVerifier {
    pub fn is_req_valid(&self, headers: &HeaderMap, data: &[u8]) -> bool {
        let values = headers
            .get_all("x-hub-signature")
            .iter()
            .collect::<Vec<_>>();

        if values.len() != 1 {
            error!("Expected to find exactly one signature header");
            return false;
        }

        let sig_header = String::from_utf8_lossy(values[0].as_bytes()).into_owned();
        self.is_valid(data, &sig_header)
    }

    pub fn is_valid(&self, data: &[u8], signature: &str) -> bool {
        // assume it starts with 'sha1='
        if signature.len() < 6 {
            error!("Invalid signature value: {}", signature);
            return false;
        }
        let sig_prefix = &signature[0..5];
        if sig_prefix != "sha1=" {
            error!("Invalid signature value. Expected sha1: {}", signature);
            return false;
        }

        let sig_bytes: Vec<u8> = match signature[5..].from_hex() {
            Ok(s) => s,
            Err(e) => {
                error!("Invalid hex value. {}", e);
                return false;
            }
        };

        let key = hmac::Key::new(hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY, self.secret.as_bytes());
        match hmac::verify(&key, data, &sig_bytes) {
            Ok(_) => {
                debug!("Signature verified!");
                true
            }
            Err(e) => {
                error!("Signature verify failed: {}", e);
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::hmac;
    use rustc_serialize::hex::ToHex;

    #[test]
    fn verify_sig_valid() {
        let key_value = String::from("this is my secret key!");
        let key = hmac::Key::new(hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY, key_value.as_bytes());

        let msg = "a message from the githubs.";
        let signature = hmac::sign(&key, msg.as_bytes());
        let signature_hex = "sha1=".to_string() + signature.as_ref().to_hex().as_str();

        let verifier = GithubWebhookVerifier {
            secret: key_value,
        };

        assert!(verifier.is_valid(msg.as_bytes(), &signature_hex));
    }

    #[test]
    fn verify_sig_wrong_digest() {
        let key_value = String::from("this is my secret key!");
        let key = hmac::Key::new(hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY, key_value.as_bytes());

        let msg = "a message from the githubs.";
        let signature = hmac::sign(&key, msg.as_bytes());
        let signature_hex = "sha9=".to_string() + signature.as_ref().to_hex().as_str();

        let verifier = GithubWebhookVerifier {
            secret: key_value,
        };

        assert!(!verifier.is_valid(msg.as_bytes(), &signature_hex));
    }

    #[test]
    fn verify_sig_missing_header() {
        let key_value = String::from("this is my secret key!");
        let key = hmac::Key::new(hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY, key_value.as_bytes());

        let msg = "a message from the githubs.";
        let signature = hmac::sign(&key, msg.as_bytes());
        let signature_hex = signature.as_ref().to_hex();

        let verifier = GithubWebhookVerifier {
            secret: key_value,
        };

        assert!(!verifier.is_valid(msg.as_bytes(), &signature_hex));
    }
}
