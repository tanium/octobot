use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use log::warn;
use ssh_key::{PublicKey, SshSig};
use tokio::time::Instant;

use octobot_events::NAMESPACE;
use octobot_lib::github::Login;

use super::{AuthFailure, Directory};

// Also how often an open stream rechecks that its user and key still exist.
pub(super) const KEYS_TTL: Duration = Duration::from_secs(5 * 60);
// Minimum time between GitHub key lookups for one login.
const KEYS_REFETCH_FLOOR: Duration = Duration::from_secs(30);
// Short of the heartbeat interval, since a stream's recheck waits on it.
const KEYS_LOOKUP_TIMEOUT: Duration = Duration::from_secs(10);

/// Removing a key on GitHub revokes it here within `KEYS_TTL`.
pub(super) struct KeyCache {
    directory: Arc<dyn Directory>,
    logins: Mutex<HashMap<Login, Entry>>,
}

#[derive(Default)]
struct Entry {
    // One lookup at a time; callers that arrive mid-lookup share its result.
    lookup: Arc<tokio::sync::Mutex<()>>,
    // When the last lookup finished, and whether it failed
    looked_up: Option<Instant>,
    failed: bool,
    // From the last lookup that succeeded, which a failed one since does not replace
    keys: Option<CachedKeys>,
}

#[derive(Clone)]
struct CachedKeys {
    fetched: Instant,
    keys: Arc<Vec<PublicKey>>,
}

impl KeyCache {
    pub(super) fn new(directory: Arc<dyn Directory>) -> KeyCache {
        KeyCache {
            directory,
            logins: Mutex::new(HashMap::new()),
        }
    }

    /// The user's key that made `sig`.
    pub(super) async fn verify(
        self: &Arc<Self>,
        login: &Login,
        msg: &[u8],
        sig: &SshSig,
    ) -> Result<PublicKey, AuthFailure> {
        let signer = |keys: &[PublicKey]| {
            keys.iter()
                .find(|k| k.verify(NAMESPACE, msg, sig).is_ok())
                .cloned()
        };

        let cached = self.cached(login);
        if let Some(key) = cached.as_ref().and_then(|keys| signer(keys)) {
            return Ok(key);
        }

        // The key may have been added on GitHub since the cached lookup.
        match self.fetch(login).await.map_err(AuthFailure::KeyLookup)? {
            Some(keys) => signer(&keys).ok_or(AuthFailure::Signature),
            None if cached.is_some() => Err(AuthFailure::Signature),
            None => Err(AuthFailure::KeyLookup(too_soon(login))),
        }
    }

    /// Whether `key` is still one of the user's keys on GitHub.
    pub(super) async fn has(
        self: &Arc<Self>,
        login: &Login,
        key: &PublicKey,
    ) -> anyhow::Result<bool> {
        let keys = self.current(login).await?;
        Ok(keys.iter().any(|k| k.key_data() == key.key_data()))
    }

    /// The user's keys, from the cache while it is fresh.
    pub(super) async fn current(
        self: &Arc<Self>,
        login: &Login,
    ) -> anyhow::Result<Arc<Vec<PublicKey>>> {
        match self.cached(login) {
            Some(keys) => Ok(keys),
            None => self.fetch(login).await?.ok_or_else(|| too_soon(login)),
        }
    }

    fn cached(&self, login: &Login) -> Option<Arc<Vec<PublicKey>>> {
        self.logins
            .lock()
            .unwrap()
            .get(login)
            .and_then(|entry| entry.keys.as_ref())
            .filter(|cached| cached.fetched.elapsed() < KEYS_TTL)
            .map(|cached| cached.keys.clone())
    }

    /// Looks up the user's keys on GitHub. Within `KEYS_REFETCH_FLOOR` of the last lookup, returns
    /// its keys instead, or None if it failed, so neither bad signatures nor GitHub errors cause
    /// a lookup each time.
    async fn fetch(self: &Arc<Self>, login: &Login) -> anyhow::Result<Option<Arc<Vec<PublicKey>>>> {
        // In its own task, so a caller hanging up mid-lookup cannot keep it from being recorded.
        let cache = self.clone();
        let login = login.clone();
        tokio::spawn(async move { cache.fetch_recorded(&login).await }).await?
    }

    async fn fetch_recorded(&self, login: &Login) -> anyhow::Result<Option<Arc<Vec<PublicKey>>>> {
        let lookup = self
            .logins
            .lock()
            .unwrap()
            .entry(login.clone())
            .or_default()
            .lookup
            .clone();
        let _one_at_a_time = lookup.lock().await;

        let recent = {
            let logins = self.logins.lock().unwrap();
            let entry = &logins[login];
            let recent = entry
                .looked_up
                .is_some_and(|at| at.elapsed() < KEYS_REFETCH_FLOOR);
            recent.then(|| match entry.failed {
                true => None,
                false => entry.keys.as_ref().map(|cached| cached.keys.clone()),
            })
        };
        if let Some(keys) = recent {
            return Ok(keys);
        }

        // octobot's GitHub client sets no timeout, and a stalled lookup would hold the lock.
        let result = tokio::time::timeout(KEYS_LOOKUP_TIMEOUT, self.lookup(login))
            .await
            .unwrap_or_else(|_| Err(anyhow::anyhow!("key lookup for {} timed out", login)));
        let now = Instant::now();
        let mut logins = self.logins.lock().unwrap();
        let entry = logins.get_mut(login).expect("entry made above");
        entry.looked_up = Some(now);
        entry.failed = result.is_err();
        if let Ok(keys) = &result {
            entry.keys = Some(CachedKeys {
                fetched: now,
                keys: keys.clone(),
            });
        }
        result.map(Some)
    }

    async fn lookup(&self, login: &Login) -> anyhow::Result<Arc<Vec<PublicKey>>> {
        let keys: Vec<PublicKey> = self
            .directory
            .keys(login)
            .await?
            .iter()
            .filter_map(|key| match PublicKey::from_openssh(key) {
                Ok(key) => Some(key),
                Err(e) => {
                    warn!("Skipping unparseable SSH key for {}: {}", login, e);
                    None
                }
            })
            .collect();
        Ok(Arc::new(keys))
    }
}

fn too_soon(login: &Login) -> anyhow::Error {
    anyhow::anyhow!("keys for {} were looked up too recently", login)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use ssh_key::{HashAlg, PrivateKey};

    use super::*;
    use crate::server::events::tests::{FakeDirectory, joe, new_key};

    fn sign(key: &PrivateKey) -> SshSig {
        key.sign(NAMESPACE, HashAlg::Sha512, b"msg").unwrap()
    }

    #[tokio::test(start_paused = true)]
    async fn test_key_added_on_github_is_found_after_refetch_floor() {
        let directory = Arc::new(FakeDirectory::default());
        let old = new_key();
        directory.set("joe", &[&old]);
        let cache = Arc::new(KeyCache::new(directory.clone()));
        assert!(cache.verify(&joe(), b"msg", &sign(&old)).await.is_ok());

        let new = new_key();
        directory.set("joe", &[&old, &new]);
        let result = cache.verify(&joe(), b"msg", &sign(&new)).await;
        assert!(matches!(result, Err(AuthFailure::Signature)));
        assert_eq!(1, directory.lookups.load(Ordering::SeqCst));

        tokio::time::advance(KEYS_REFETCH_FLOOR).await;
        assert!(cache.verify(&joe(), b"msg", &sign(&new)).await.is_ok());
    }

    #[tokio::test(start_paused = true)]
    async fn test_failed_lookups_are_not_retried_within_refetch_floor() {
        let directory = Arc::new(FakeDirectory::default());
        let key = new_key();
        directory.set("joe", &[&key]);
        directory.failing.store(true, Ordering::SeqCst);
        let cache = Arc::new(KeyCache::new(directory.clone()));

        for _ in 0..3 {
            let result = cache.verify(&joe(), b"msg", &sign(&key)).await;
            assert!(matches!(result, Err(AuthFailure::KeyLookup(_))));
        }
        assert_eq!(1, directory.lookups.load(Ordering::SeqCst));

        directory.failing.store(false, Ordering::SeqCst);
        tokio::time::advance(KEYS_REFETCH_FLOOR).await;
        assert!(cache.verify(&joe(), b"msg", &sign(&key)).await.is_ok());
    }

    #[tokio::test(start_paused = true)]
    async fn test_concurrent_verifies_share_one_lookup() {
        let directory = Arc::new(FakeDirectory::default());
        let key = new_key();
        directory.set("joe", &[&key]);
        *directory.delay.lock().unwrap() = Duration::from_secs(1);
        let cache = Arc::new(KeyCache::new(directory.clone()));

        let sig = sign(&key);
        let login = joe();
        let (first, second) = tokio::join!(
            cache.verify(&login, b"msg", &sig),
            cache.verify(&login, b"msg", &sig)
        );
        assert!(first.is_ok());
        assert!(second.is_ok());
        assert_eq!(1, directory.lookups.load(Ordering::SeqCst));
    }

    // A caller hanging up mid-lookup must neither skip the refetch floor nor block the next caller.
    #[tokio::test(start_paused = true)]
    async fn test_cancelled_verify_still_records_its_lookup() {
        let directory = Arc::new(FakeDirectory::default());
        let key = new_key();
        directory.set("joe", &[&key]);
        *directory.delay.lock().unwrap() = Duration::from_secs(10);
        let cache = Arc::new(KeyCache::new(directory.clone()));

        let sig = sign(&key);
        let cancelled =
            tokio::time::timeout(Duration::from_secs(1), cache.verify(&joe(), b"msg", &sig)).await;
        assert!(cancelled.is_err());

        assert!(cache.verify(&joe(), b"msg", &sig).await.is_ok());
        assert_eq!(1, directory.lookups.load(Ordering::SeqCst));
    }

    #[tokio::test(start_paused = true)]
    async fn test_stalled_lookup_times_out_and_counts() {
        let directory = Arc::new(FakeDirectory::default());
        let key = new_key();
        directory.set("joe", &[&key]);
        *directory.delay.lock().unwrap() = Duration::from_secs(3600);
        let cache = Arc::new(KeyCache::new(directory.clone()));

        let sig = sign(&key);
        for _ in 0..2 {
            let result = cache.verify(&joe(), b"msg", &sig).await;
            assert!(matches!(result, Err(AuthFailure::KeyLookup(_))));
        }
        assert_eq!(1, directory.lookups.load(Ordering::SeqCst));
    }
}
