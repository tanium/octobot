use std::sync::RwLock;
use std::time::{Duration, Instant};

use ring::rand::SecureRandom;
use ring::rand::SystemRandom;
use rustc_serialize::hex::ToHex;

static SESSION_EXPIRY_SECS: u64 = 15 * 60;
static PRUNE_SECS: u64 = 30;

pub struct Sessions {
    sessions: RwLock<Vec<Session>>,
    last_pruned: RwLock<Instant>,
}

struct Session {
    id: String,
    // Note: could change this to last_accessed, but then we'd have to worry about max
    // session time too. Keep it simple for now.
    created_at: Instant,
}

impl Sessions {
    pub fn new() -> Sessions {
        Sessions {
            sessions: RwLock::new(vec![]),
            last_pruned: RwLock::new(Instant::now()),
        }
    }

    pub fn new_session(&self) -> String {
        let mut bytes: [u8; 32] = [0; 32];
        // Doesn't look like SecureRandom, but docs claim it is.
        SystemRandom::new().fill(&mut bytes).expect("get random");

        let sess_id = bytes.to_hex();
        let session = Session {
            id: sess_id.clone(),
            created_at: Instant::now(),
        };

        self.sessions.write().unwrap().push(session);

        sess_id
    }

    pub fn remove_session(&self, sess_id: &str) {
        let mut sessions = self.sessions.write().unwrap();
        sessions.retain(|s| s.id != sess_id);
    }

    pub fn is_valid_session(&self, sess_id: &str) -> bool {
        self.prune(); // maybe prune out old sessions first

        let sessions = self.sessions.read().unwrap();
        sessions.iter().any(|s| s.id == sess_id)
    }

    fn needs_prune(&self) -> bool {
        let last_pruned = self.last_pruned.read().unwrap();
        last_pruned.elapsed() >= Duration::from_secs(PRUNE_SECS)
    }

    fn prune(&self) {
        if self.needs_prune() {
            let mut last_pruned = self.last_pruned.write().unwrap();

            let mut sessions = self.sessions.write().unwrap();
            sessions.retain(|s| s.created_at.elapsed() < Duration::from_secs(SESSION_EXPIRY_SECS));

            *last_pruned = Instant::now();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sessions() {
        let sessions = Sessions::new();
        let sess1 = sessions.new_session();
        let sess2 = sessions.new_session();

        assert_eq!(true, sessions.is_valid_session(&sess1));
        assert_eq!(true, sessions.is_valid_session(&sess2));

        sessions.remove_session(&sess1);

        assert_eq!(false, sessions.is_valid_session(&sess1));
        assert_eq!(true, sessions.is_valid_session(&sess2));

        sessions.remove_session(&sess2);

        assert_eq!(false, sessions.is_valid_session(&sess2));
    }

    #[test]
    fn test_sessions_timeout() {
        let sessions = Sessions::new();

        let sess = sessions.new_session();
        assert_eq!(true, sessions.is_valid_session(&sess));

        // reset only last prune time. not enough.
        *sessions.last_pruned.write().unwrap() -= Duration::from_secs(PRUNE_SECS + 1);
        assert_eq!(true, sessions.is_valid_session(&sess));

        // reset only last prune time and last accessed time
        *sessions.last_pruned.write().unwrap() -= Duration::from_secs(PRUNE_SECS + 1);
        sessions.sessions.write().unwrap()[0].created_at -=
            Duration::from_secs(SESSION_EXPIRY_SECS + 1);
        assert_eq!(false, sessions.is_valid_session(&sess));
    }
}
