use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use log::{info, warn};
use ring::hmac;
use ring::rand::{SecureRandom, SystemRandom};
use ssh_key::{PublicKey, SshSig};
use tokio::sync::{broadcast, mpsc};
use tokio::time::Instant;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};

use octobot_events::NAMESPACE;
use octobot_events::pb::events_server::Events;
use octobot_events::pb::{self, ChallengeRequest, ChallengeResponse, SubscribeRequest, event};
use octobot_lib::config::Config;
use octobot_lib::github::Login;
use octobot_lib::github::api::GithubSessionFactory;
use octobot_lib::metrics::{self, Metrics, ScopedGaugeCounter};
use octobot_ops::event_bus::{Event, EventBus, Kind};

const CHALLENGE_TTL: Duration = Duration::from_secs(60);
// Also how often an open stream rechecks that its user and key still exist.
const KEYS_TTL: Duration = Duration::from_secs(5 * 60);
// Minimum time between GitHub key lookups for one login.
const KEYS_REFETCH_FLOOR: Duration = Duration::from_secs(30);
// Short of the heartbeat interval, since a stream's recheck waits on it.
const KEYS_LOOKUP_TIMEOUT: Duration = Duration::from_secs(10);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
const MAX_STREAMS_PER_USER: usize = 10;
const MAX_STREAMS: usize = 1_000;
// Small, so a stuck client lags on the bus and is aborted instead of buffering here.
const STREAM_BUFFER: usize = 64;

/// Serves the event bus to octobot users who prove they hold an SSH key registered to their
/// GitHub login. Each stream carries only the changes octobot notified that login about.
pub struct EventsService {
    bus: Arc<EventBus>,
    authority: String,
    directory: Arc<dyn Directory>,
    challenges: Challenges,
    keys: Arc<KeyCache>,
    streams: Arc<StreamCounts>,
    metrics: Arc<Metrics>,
}

impl EventsService {
    pub fn new(
        bus: Arc<EventBus>,
        authority: &str,
        directory: Arc<dyn Directory>,
        metrics: Arc<Metrics>,
    ) -> anyhow::Result<EventsService> {
        // Clients connect through the TLS proxy, so they dial https.
        let uri = format!("https://{}", authority).parse()?;
        Ok(EventsService {
            bus,
            authority: octobot_events::signed_authority(&uri)?,
            directory: directory.clone(),
            challenges: Challenges::new(),
            keys: Arc::new(KeyCache::new(directory)),
            streams: Arc::new(StreamCounts::new()),
            metrics,
        })
    }

    /// Only octobot users get hints. The lookup also yields a login safe to put in a GitHub API
    /// path, and refusing others keeps strangers from making octobot look up arbitrary logins.
    fn octobot_user(&self, username: &str) -> Result<Login, AuthFailure> {
        self.directory
            .user(username)
            .map_err(AuthFailure::UserLookup)?
            .ok_or(AuthFailure::NotUser)
    }

    async fn authenticate(&self, auth: Option<&pb::Auth>) -> Result<(Login, PublicKey), Status> {
        let result = self.verify(auth).await;
        if let Err(failure) = &result {
            self.metrics
                .event_stream_auth_failures
                .with_label_values(&[failure.reason()])
                .inc();
        }
        result.map_err(AuthFailure::status)
    }

    async fn verify(&self, auth: Option<&pb::Auth>) -> Result<(Login, PublicKey), AuthFailure> {
        let auth = auth.ok_or(AuthFailure::Malformed("missing auth"))?;
        let login = self.octobot_user(&auth.username)?;
        let sig = SshSig::from_pem(&auth.signature)
            .map_err(|_| AuthFailure::Malformed("bad signature"))?;

        let expiry = self
            .challenges
            .check(&auth.nonce, login.as_str())
            .ok_or(AuthFailure::Nonce)?;
        let msg = octobot_events::signed_message(&self.authority, &auth.nonce);
        let key = self.keys.verify(&login, &msg, &sig).await?;
        if !self.challenges.consume(&auth.nonce, expiry) {
            return Err(AuthFailure::Nonce);
        }
        Ok((login, key))
    }
}

#[tonic::async_trait]
impl Events for EventsService {
    async fn challenge(
        &self,
        req: Request<ChallengeRequest>,
    ) -> Result<Response<ChallengeResponse>, Status> {
        let login = self
            .octobot_user(&req.into_inner().username)
            .map_err(AuthFailure::status)?;
        let (nonce, expires) = self.challenges.issue(login.as_str())?;
        Ok(Response::new(ChallengeResponse {
            nonce,
            expires_unix: expires.duration_since(UNIX_EPOCH).unwrap().as_secs() as i64,
        }))
    }

    type SubscribeStream = ReceiverStream<Result<pb::Event, Status>>;

    async fn subscribe(
        &self,
        req: Request<SubscribeRequest>,
    ) -> Result<Response<Self::SubscribeStream>, Status> {
        let req = req.into_inner();
        let (login, key) = self.authenticate(req.auth.as_ref()).await?;
        let slot = StreamCounts::open(&self.streams, login.as_str())?;
        info!("Opening event stream for {}", login);

        let stream = Stream {
            login,
            key,
            directory: self.directory.clone(),
            keys: self.keys.clone(),
            kinds: req.kinds().collect(),
            metrics: self.metrics.clone(),
            _slot: slot,
            _count: metrics::scoped_inc(&self.metrics.current_event_stream_count),
        };
        // Subscribing before answering lets the client load state once it has the answer
        // without missing a change.
        let receiver = self.bus.subscribe();
        let (tx, rx) = mpsc::channel(STREAM_BUFFER);
        tokio::spawn(stream.run(receiver, tx));
        Ok(Response::new(ReceiverStream::new(rx)))
    }
}

enum AuthFailure {
    Malformed(&'static str),
    NotUser,
    UserLookup(anyhow::Error),
    Nonce,
    Signature,
    KeyLookup(anyhow::Error),
}

impl AuthFailure {
    fn reason(&self) -> &'static str {
        match self {
            AuthFailure::Malformed(_) => "malformed",
            AuthFailure::NotUser => "not_user",
            AuthFailure::UserLookup(_) => "user_lookup",
            AuthFailure::Nonce => "nonce",
            AuthFailure::Signature => "signature",
            AuthFailure::KeyLookup(_) => "key_lookup",
        }
    }

    fn status(self) -> Status {
        match self {
            AuthFailure::Malformed(msg) => Status::unauthenticated(msg),
            AuthFailure::NotUser => Status::permission_denied("not an octobot user"),
            AuthFailure::UserLookup(e) => {
                warn!("Error looking up octobot user: {:#}", e);
                Status::unavailable("could not look up octobot users")
            }
            AuthFailure::Nonce => Status::unauthenticated("unknown or expired challenge"),
            AuthFailure::Signature => {
                Status::unauthenticated("signature does not match the user's GitHub keys")
            }
            AuthFailure::KeyLookup(e) => {
                warn!("Error looking up SSH keys: {:#}", e);
                Status::unavailable("could not look up the user's GitHub keys")
            }
        }
    }
}

/// Single-use nonces, each bound to the login it was issued for. A nonce carries its own expiry
/// and an HMAC over it and the login, so issuing one stores nothing. Only nonces that
/// authenticated a stream are remembered, until they expire, to refuse replays.
struct Challenges {
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
    fn new() -> Challenges {
        let random = SystemRandom::new();
        let key = hmac::Key::generate(hmac::HMAC_SHA256, &random).expect("generate nonce key");
        Challenges {
            random,
            key,
            epoch: Instant::now(),
            used: Mutex::new(HashMap::new()),
        }
    }

    fn issue(&self, login: &str) -> Result<(Vec<u8>, SystemTime), Status> {
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
    fn check(&self, nonce: &[u8], login: &str) -> Option<Instant> {
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
    fn consume(&self, nonce: &[u8], expiry: Instant) -> bool {
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

/// Who may subscribe, and with which keys.
#[async_trait]
pub trait Directory: Send + Sync {
    /// The login of the octobot user named `username`, if there is one.
    fn user(&self, username: &str) -> anyhow::Result<Option<Login>>;
    /// The user's public SSH keys on GitHub, in OpenSSH format.
    async fn keys(&self, login: &Login) -> anyhow::Result<Vec<String>>;
}

pub struct GithubDirectory {
    pub config: Arc<Config>,
    pub github: Arc<dyn GithubSessionFactory>,
}

#[async_trait]
impl Directory for GithubDirectory {
    fn user(&self, username: &str) -> anyhow::Result<Option<Login>> {
        self.config.users().github_login(username)
    }

    async fn keys(&self, login: &Login) -> anyhow::Result<Vec<String>> {
        self.github.get_user_keys(login).await
    }
}

/// Removing a key on GitHub revokes it here within `KEYS_TTL`.
struct KeyCache {
    directory: Arc<dyn Directory>,
    entries: Mutex<HashMap<Login, CachedKeys>>,
    // Last lookup per login, failed or not
    attempts: Mutex<HashMap<Login, Attempt>>,
    // One lookup per login at a time; callers that arrive mid-lookup share its result.
    in_flight: Mutex<HashMap<Login, Arc<tokio::sync::Mutex<()>>>>,
}

#[derive(Clone)]
struct CachedKeys {
    fetched: Instant,
    keys: Arc<Vec<PublicKey>>,
}

#[derive(Clone)]
struct Attempt {
    at: Instant,
    /// None if the lookup failed
    keys: Option<Arc<Vec<PublicKey>>>,
}

impl KeyCache {
    fn new(directory: Arc<dyn Directory>) -> KeyCache {
        KeyCache {
            directory,
            entries: Mutex::new(HashMap::new()),
            attempts: Mutex::new(HashMap::new()),
            in_flight: Mutex::new(HashMap::new()),
        }
    }

    /// The user's key that made `sig`.
    async fn verify(
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
        if let Some(key) = cached.as_ref().and_then(|cached| signer(&cached.keys)) {
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
    async fn has(self: &Arc<Self>, login: &Login, key: &PublicKey) -> anyhow::Result<bool> {
        let keys = match self.cached(login) {
            Some(cached) => cached.keys,
            None => self.fetch(login).await?.ok_or_else(|| too_soon(login))?,
        };
        Ok(keys.iter().any(|k| k.key_data() == key.key_data()))
    }

    fn cached(&self, login: &Login) -> Option<CachedKeys> {
        self.entries
            .lock()
            .unwrap()
            .get(login)
            .filter(|cached| cached.fetched.elapsed() < KEYS_TTL)
            .cloned()
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
        let in_flight = self
            .in_flight
            .lock()
            .unwrap()
            .entry(login.clone())
            .or_default()
            .clone();
        let _one_at_a_time = in_flight.lock().await;

        let recent = self
            .attempts
            .lock()
            .unwrap()
            .get(login)
            .filter(|attempt| attempt.at.elapsed() < KEYS_REFETCH_FLOOR)
            .cloned();
        if let Some(attempt) = recent {
            return Ok(attempt.keys);
        }

        // octobot's GitHub client sets no timeout, and a stalled lookup would hold the lock.
        let result = tokio::time::timeout(KEYS_LOOKUP_TIMEOUT, self.lookup(login))
            .await
            .unwrap_or_else(|_| Err(anyhow::anyhow!("key lookup for {} timed out", login)));
        let keys = result.as_ref().ok().cloned();
        self.attempts.lock().unwrap().insert(
            login.clone(),
            Attempt {
                at: Instant::now(),
                keys: keys.clone(),
            },
        );
        if let Some(keys) = keys {
            self.entries.lock().unwrap().insert(
                login.clone(),
                CachedKeys {
                    fetched: Instant::now(),
                    keys,
                },
            );
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

struct StreamCounts {
    by_login: Mutex<HashMap<String, usize>>,
}

/// Holds a stream's place under the limits until dropped.
struct StreamSlot {
    counts: Arc<StreamCounts>,
    login: String,
}

impl StreamCounts {
    fn new() -> StreamCounts {
        StreamCounts {
            by_login: Mutex::new(HashMap::new()),
        }
    }

    fn open(counts: &Arc<StreamCounts>, login: &str) -> Result<StreamSlot, Status> {
        let mut by_login = counts.by_login.lock().unwrap();
        if by_login.values().sum::<usize>() >= MAX_STREAMS {
            return Err(Status::resource_exhausted("too many event streams"));
        }
        let count = by_login.entry(login.to_string()).or_default();
        if *count >= MAX_STREAMS_PER_USER {
            return Err(Status::resource_exhausted(
                "too many event streams for user",
            ));
        }
        *count += 1;
        Ok(StreamSlot {
            counts: counts.clone(),
            login: login.to_string(),
        })
    }
}

impl Drop for StreamSlot {
    fn drop(&mut self) {
        let mut by_login = self.counts.by_login.lock().unwrap();
        if let Some(count) = by_login.get_mut(&self.login) {
            *count -= 1;
            if *count == 0 {
                by_login.remove(&self.login);
            }
        }
    }
}

struct Stream {
    login: Login,
    /// The key that authenticated the stream
    key: PublicKey,
    directory: Arc<dyn Directory>,
    keys: Arc<KeyCache>,
    /// Empty means all.
    kinds: HashSet<pb::Kind>,
    metrics: Arc<Metrics>,
    _slot: StreamSlot,
    _count: ScopedGaugeCounter,
}

impl Stream {
    async fn run(
        self,
        mut receiver: broadcast::Receiver<Arc<Event>>,
        tx: mpsc::Sender<Result<pb::Event, Status>>,
    ) {
        let mut heartbeat =
            tokio::time::interval_at(Instant::now() + HEARTBEAT_INTERVAL, HEARTBEAT_INTERVAL);
        let mut recheck = tokio::time::interval_at(Instant::now() + KEYS_TTL, KEYS_TTL);
        loop {
            let body = tokio::select! {
                _ = tx.closed() => return,
                _ = heartbeat.tick() => event::Body::Heartbeat(pb::Heartbeat {}),
                _ = recheck.tick() => match self.still_allowed().await {
                    Ok(true) => continue,
                    Ok(false) => {
                        info!("Revoking event stream for {}", self.login);
                        let revoked = Status::unauthenticated("user or key no longer allowed");
                        let _ = tx.send(Err(revoked)).await;
                        return;
                    }
                    // Revoking takes a positive answer; a GitHub outage must not drop every
                    // stream at once.
                    Err(e) => {
                        warn!("Error rechecking event stream for {}: {:#}", self.login, e);
                        continue;
                    }
                },
                received = receiver.recv() => match received {
                    Ok(event) => {
                        if !self.wants(&event) {
                            continue;
                        }
                        event::Body::Change(pb::Change {
                            repo: event.change.repo.clone(),
                            kind: pb_kind(event.change.kind) as i32,
                            action: event.change.action.clone(),
                            number: event.change.number,
                        })
                    }
                    // Changes were dropped. Ending the stream makes the client reconnect and
                    // reload, the same as after any other disconnect.
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        info!("Aborting lagging event stream for {}", self.login);
                        self.metrics.event_stream_messages.with_label_values(&["aborted"]).inc();
                        let _ = tx.send(Err(Status::aborted("stream fell behind"))).await;
                        return;
                    }
                    Err(broadcast::error::RecvError::Closed) => return,
                },
            };
            let kind = match body {
                event::Body::Change(_) => "change",
                event::Body::Heartbeat(_) => "heartbeat",
            };
            if tx.send(Ok(pb::Event { body: Some(body) })).await.is_err() {
                return;
            }
            self.metrics
                .event_stream_messages
                .with_label_values(&[kind])
                .inc();
        }
    }

    async fn still_allowed(&self) -> anyhow::Result<bool> {
        let user = self.directory.user(self.login.as_str())?;
        Ok(user.is_some() && self.keys.has(&self.login, &self.key).await?)
    }

    fn wants(&self, event: &Event) -> bool {
        event.is_for(self.login.as_str())
            && (self.kinds.is_empty() || self.kinds.contains(&pb_kind(event.change.kind)))
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        info!("Closed event stream for {}", self.login);
    }
}

fn pb_kind(kind: Kind) -> pb::Kind {
    match kind {
        Kind::PullRequest => pb::Kind::PullRequest,
        Kind::PullRequestReview => pb::Kind::PullRequestReview,
        Kind::PullRequestReviewComment => pb::Kind::PullRequestReviewComment,
        Kind::CommitComment => pb::Kind::CommitComment,
        Kind::IssueComment => pb::Kind::IssueComment,
        Kind::Push => pb::Kind::Push,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use ssh_key::rand_core::OsRng;
    use ssh_key::{Algorithm, HashAlg, PrivateKey};

    use super::*;

    #[derive(Default)]
    struct FakeDirectory {
        keys: Mutex<HashMap<String, Vec<String>>>,
        removed_users: Mutex<HashSet<String>>,
        lookups: AtomicUsize,
        failing: AtomicBool,
        delay: Mutex<Duration>,
    }

    impl FakeDirectory {
        fn set(&self, login: &str, keys: &[&PrivateKey]) {
            let keys = keys
                .iter()
                .map(|k| k.public_key().to_openssh().unwrap())
                .collect();
            self.keys.lock().unwrap().insert(login.to_string(), keys);
        }

        /// Removes the octobot user, leaving their GitHub keys.
        fn remove_user(&self, login: &str) {
            self.removed_users.lock().unwrap().insert(login.to_string());
        }
    }

    #[async_trait]
    impl Directory for FakeDirectory {
        fn user(&self, username: &str) -> anyhow::Result<Option<Login>> {
            // Like the users table, matches case-insensitively before parsing what it stores.
            let name = username.to_lowercase();
            let is_user = self.keys.lock().unwrap().contains_key(&name)
                && !self.removed_users.lock().unwrap().contains(&name);
            is_user.then(|| Login::parse(&name)).transpose()
        }

        async fn keys(&self, login: &Login) -> anyhow::Result<Vec<String>> {
            self.lookups.fetch_add(1, Ordering::SeqCst);
            let delay = *self.delay.lock().unwrap();
            tokio::time::sleep(delay).await;
            if self.failing.load(Ordering::SeqCst) {
                anyhow::bail!("GitHub is down");
            }
            Ok(self
                .keys
                .lock()
                .unwrap()
                .get(login.as_str())
                .cloned()
                .unwrap_or_default())
        }
    }

    fn joe() -> Login {
        Login::parse("joe").unwrap()
    }

    fn new_key() -> PrivateKey {
        PrivateKey::random(&mut OsRng, Algorithm::Ed25519).unwrap()
    }

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

    #[test]
    fn test_configured_authority_is_signed_form() {
        let service = EventsService::new(
            Arc::new(EventBus::new(10)),
            "Octobot.Example.com:443",
            Arc::new(FakeDirectory::default()),
            Metrics::new(),
        )
        .unwrap();
        assert_eq!("octobot.example.com", service.authority);
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

    /// Starts a stream for joe, authenticated by `key`. It ends if the bus is dropped.
    fn start_stream(
        directory: Arc<FakeDirectory>,
        key: &PrivateKey,
    ) -> (EventBus, mpsc::Receiver<Result<pb::Event, Status>>) {
        let metrics = Metrics::new();
        let counts = Arc::new(StreamCounts::new());
        let stream = Stream {
            login: joe(),
            key: key.public_key().clone(),
            directory: directory.clone(),
            // Empty, so the first recheck asks the directory.
            keys: Arc::new(KeyCache::new(directory)),
            kinds: HashSet::new(),
            metrics: metrics.clone(),
            _slot: StreamCounts::open(&counts, "joe").unwrap(),
            _count: metrics::scoped_inc(&metrics.current_event_stream_count),
        };
        let bus = EventBus::new(10);
        let (tx, rx) = mpsc::channel(STREAM_BUFFER);
        tokio::spawn(stream.run(bus.subscribe(), tx));
        (bus, rx)
    }

    async fn run_stream(
        directory: Arc<FakeDirectory>,
        key: &PrivateKey,
    ) -> Vec<Result<pb::Event, Status>> {
        let (_bus, mut rx) = start_stream(directory, key);
        let mut sent = vec![];
        let collect = async {
            while let Some(msg) = rx.recv().await {
                sent.push(msg);
            }
        };
        tokio::time::timeout(2 * KEYS_TTL, collect)
            .await
            .expect("stream still open");
        sent
    }

    // The last heartbeat and the recheck share a tick, so either may come first.
    fn assert_revoked_at_recheck(sent: Vec<Result<pb::Event, Status>>) {
        let (last, heartbeats) = sent.split_last().unwrap();
        let interval_count = (KEYS_TTL.as_secs() / HEARTBEAT_INTERVAL.as_secs()) as usize;
        assert!(heartbeats.len() >= interval_count - 1);
        for msg in heartbeats {
            let body = msg.as_ref().unwrap().body.clone();
            assert_eq!(Some(event::Body::Heartbeat(pb::Heartbeat {})), body);
        }
        let status = last.as_ref().unwrap_err();
        assert_eq!(tonic::Code::Unauthenticated, status.code());
    }

    #[tokio::test(start_paused = true)]
    async fn test_stream_closes_when_key_is_removed() {
        let directory = Arc::new(FakeDirectory::default());
        let key = new_key();
        directory.set("joe", &[&new_key()]);

        assert_revoked_at_recheck(run_stream(directory, &key).await);
    }

    #[tokio::test(start_paused = true)]
    async fn test_stream_closes_when_user_is_removed() {
        let directory = Arc::new(FakeDirectory::default());
        let key = new_key();
        directory.set("joe", &[&key]);
        directory.remove_user("joe");

        assert_revoked_at_recheck(run_stream(directory, &key).await);
    }

    #[tokio::test(start_paused = true)]
    async fn test_stream_stays_open_while_allowed() {
        let directory = Arc::new(FakeDirectory::default());
        let key = new_key();
        directory.set("joe", &[&key]);
        let (_bus, mut rx) = start_stream(directory, &key);

        // Past two rechecks.
        let heartbeats = 2 * KEYS_TTL.as_secs() / HEARTBEAT_INTERVAL.as_secs() + 1;
        for _ in 0..heartbeats {
            let body = rx.recv().await.unwrap().unwrap().body;
            assert_eq!(Some(event::Body::Heartbeat(pb::Heartbeat {})), body);
        }
    }

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

    #[test]
    fn test_streams_per_user_are_limited() {
        let counts = Arc::new(StreamCounts::new());
        let mut slots: Vec<StreamSlot> = (0..MAX_STREAMS_PER_USER)
            .map(|_| StreamCounts::open(&counts, "joe").unwrap())
            .collect();

        assert!(StreamCounts::open(&counts, "joe").is_err());
        assert!(StreamCounts::open(&counts, "bob").is_ok());

        slots.pop();
        assert!(StreamCounts::open(&counts, "joe").is_ok());
    }
}
