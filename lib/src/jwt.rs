use std::time::{SystemTime, UNIX_EPOCH};

use jsonwebtoken::{self, Algorithm, Header, EncodingKey};
use serde_derive::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    iat: u64,
    exp: u64,
    iss: String,
}

pub fn new_token(app_id: u32, app_key_der: &[u8]) -> String {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
    let claims = Claims {
        iat: now,
        exp: now + (9 * 60),
        iss: app_id.to_string(),
    };

    let key = EncodingKey::from_rsa_der(app_key_der);

    return jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, &key).unwrap();
}
