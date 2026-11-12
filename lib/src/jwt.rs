use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::anyhow;
use jsonwebtoken::{self, Algorithm, EncodingKey, Header};
use log;
use serde_derive::{Deserialize, Serialize};

use crate::errors::*;

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    iat: u64,
    exp: u64,
    iss: String,
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

pub fn encoding_key(app_key_bytes: &[u8]) -> EncodingKey {
    match EncodingKey::from_rsa_pem(app_key_bytes) {
        Ok(k) => k,
        Err(e) => {
            log::info!("Expected RSA keyin PEM format: {}. Falling back to DER.", e);
            EncodingKey::from_rsa_der(app_key_bytes)
        }
    }
}

pub fn new_token(app_id: u32, key: &EncodingKey, now: u64) -> Result<String> {
    let claims = Claims {
        // Backdated so a host clock running slightly fast is not rejected.
        iat: now - 60,
        exp: now + (9 * 60),
        iss: app_id.to_string(),
    };

    jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, key)
        .map_err(|e| anyhow!("Failed to create JWT: {}", e))
}
