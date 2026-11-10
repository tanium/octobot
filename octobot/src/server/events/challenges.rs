use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use ring::hmac;
use ring::rand::{SecureRandom, SystemRandom};
use tokio::time::Instant;
use tonic::Status;

const CHALLENGE_TTL: Duration = Duration::from_secs(60);

/// Single-use nonces, each bound to the login it was issued for. A nonce carries its own expiry
/// and an HMAC over it and the login, so issuing one stores nothing. Only nonces that
/// authenticated a stream are remembered, until they expire, to refuse replays.
pub(super) struct Challenges {
    random: SystemRandom,
    key: hmac::Key,
    // Expiries are encoded relative to this.
    epoch: Instant,
    used: Mutex<HashMap<Vec<u8>, Instant>>,
}

// expiry millis (8) | random (16) | HMAC-SHA256 tag (32)
const NONCE_BODY_LEN: usize = 8 + 16;
const NONCE_LEN: usize = NONCE_BODY_LEN + 32;

impl Challenges {
    pub(super) fn new() -> Challenges {
        let random = SystemRandom::new();
        let key = hmac::Key::generate(hmac::HMAC_SHA256, &random).expect("generate nonce key");
        Challenges {
            random,
            key,
            epoch: Instant::now(),
            used: Mutex::new(HashMap::new()),
        }
    }

    pub(super) fn issue(&self, login: &str) -> Result<(Vec<u8>, SystemTime), Status> {
        let expiry = Instant::now() + CHALLENGE_TTL - self.epoch;
        let mut nonce = (expiry.as_millis() as u64).to_be_bytes().to_vec();
        let mut random = [0; 16];
        self.random
            .fill(&mut random)
            .map_err(|_| Status::internal("could not generate nonce"))?;
        nonce.extend_from_slice(&random);
        let tag = hmac::sign(&self.key, &tagged(login, &nonce));
        nonce.extend_from_slice(tag.as_ref());
        Ok((nonce, SystemTime::now() + CHALLENGE_TTL))
    }

    /// The nonce's expiry, if it was issued here to `login` and has not expired.
    pub(super) fn check(&self, nonce: &[u8], login: &str) -> Option<Instant> {
        if nonce.len() != NONCE_LEN {
            return None;
        }
        let (body, tag) = nonce.split_at(NONCE_BODY_LEN);
        hmac::verify(&self.key, &tagged(login, body), tag).ok()?;
        let millis = u64::from_be_bytes(body[..8].try_into().unwrap());
        let expiry = self.epoch + Duration::from_millis(millis);
        (expiry > Instant::now()).then_some(expiry)
    }

    /// Marks a checked nonce used; false if it already was.
    pub(super) fn consume(&self, nonce: &[u8], expiry: Instant) -> bool {
        let now = Instant::now();
        let mut used = self.used.lock().unwrap();
        used.retain(|_, expiry| *expiry > now);
        used.insert(nonce.to_vec(), expiry).is_none()
    }
}

// Logins never contain a NUL, so the login and body cannot run together ambiguously.
fn tagged(login: &str, body: &[u8]) -> Vec<u8> {
    let mut msg = login.as_bytes().to_vec();
    msg.push(0);
    msg.extend_from_slice(body);
    msg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn test_challenge_expires() {
        let challenges = Challenges::new();
        let (nonce, _) = challenges.issue("joe").unwrap();

        tokio::time::advance(CHALLENGE_TTL - Duration::from_secs(1)).await;
        assert!(challenges.check(&nonce, "joe").is_some());

        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(challenges.check(&nonce, "joe").is_none());
    }

    #[tokio::test]
    async fn test_challenge_is_bound_to_login() {
        let challenges = Challenges::new();
        let (nonce, _) = challenges.issue("joe").unwrap();

        assert!(challenges.check(&nonce, "bob").is_none());
    }

    #[tokio::test]
    async fn test_challenge_rejects_tampering() {
        let challenges = Challenges::new();
        let (nonce, _) = challenges.issue("joe").unwrap();
        let mut later = nonce.clone();
        later[0] ^= 1;

        assert!(challenges.check(&later, "joe").is_none());
        assert!(Challenges::new().check(&nonce, "joe").is_none());
    }

    #[tokio::test]
    async fn test_challenge_is_single_use() {
        let challenges = Challenges::new();
        let (nonce, _) = challenges.issue("joe").unwrap();
        let expiry = challenges.check(&nonce, "joe").unwrap();

        assert!(challenges.consume(&nonce, expiry));
        assert!(!challenges.consume(&nonce, expiry));
    }
}
