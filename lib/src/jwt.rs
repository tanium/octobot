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

pub fn new_token(app_id: u32, app_key_bytes: &[u8]) -> Result<String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let claims = Claims {
        iat: now,
        exp: now + (9 * 60),
        iss: app_id.to_string(),
    };

    let key = match EncodingKey::from_rsa_pem(app_key_bytes) {
        Ok(k) => k,
        Err(e) => {
            log::info!("Expected RSA keyin PEM format: {}. Falling back to DER.", e);
            EncodingKey::from_rsa_der(app_key_bytes)
        }
    };

    jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, &key)
        .map_err(|e| anyhow!("Failed to create JWT: {}", e))
}
