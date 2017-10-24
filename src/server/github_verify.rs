use hyper::header::Headers;
use ring::{digest, hmac};
use rustc_serialize::hex::FromHex;

pub struct GithubWebhookVerifier {
    pub secret: String,
}

impl GithubWebhookVerifier {
    pub fn is_req_valid(&self, headers: &Headers, data: &[u8]) -> bool {
        let sig_header: String = match headers.get_raw("x-hub-signature") {
            Some(ref h) if h.len() == 1 => String::from_utf8_lossy(&h[0]).into_owned(),
            None | Some(..) => {
                error!("Expected to find exactly one signature header");
                return false;
            }
        };

        return self.is_valid(data, &sig_header);
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

        let key = hmac::VerificationKey::new(&digest::SHA1, self.secret.as_bytes());
        match hmac::verify(&key, data, &sig_bytes) {
            Ok(_) => {
                debug!("Signature verified!");
                true
            },
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

        assert!(verifier.is_valid(msg.as_bytes(), &signature_hex));
    }

    #[test]
    fn verify_sig_wrong_digest() {
        let key_value = String::from("this is my secret key!");
        let key = hmac::SigningKey::new(&digest::SHA1, key_value.as_bytes());

        let msg = "a message from the githubs.";
        let signature = hmac::sign(&key, msg.as_bytes());
        let signature_hex = "sha9=".to_string() + signature.as_ref().to_hex().as_str();

        let verifier = GithubWebhookVerifier { secret: key_value.clone() };

        assert!(!verifier.is_valid(msg.as_bytes(), &signature_hex));
    }

    #[test]
    fn verify_sig_missing_header() {
        let key_value = String::from("this is my secret key!");
        let key = hmac::SigningKey::new(&digest::SHA1, key_value.as_bytes());

        let msg = "a message from the githubs.";
        let signature = hmac::sign(&key, msg.as_bytes());
        let signature_hex = signature.as_ref().to_hex();

        let verifier = GithubWebhookVerifier { secret: key_value.clone() };

        assert!(!verifier.is_valid(msg.as_bytes(), &signature_hex));
    }
}
