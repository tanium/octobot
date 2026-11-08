use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use log::{info, warn};
use ring::rand::{SecureRandom, SystemRandom};
use ssh_key::{PublicKey, SshSig};
use tokio::sync::{broadcast, mpsc};
use tokio::time::Instant;
use tokio_stream::wrappers::ReceiverStream;
use tonic::metadata::MetadataMap;
use tonic::{Request, Response, Status};

use octobot_events::pb::events_server::Events;
use octobot_events::pb::{self, ChallengeRequest, ChallengeResponse, SubscribeRequest, event};
use octobot_events::{NAMESPACE, NONCE_HEADER, SIG_HEADER, USER_HEADER};
use octobot_lib::github::api::GithubSessionFactory;
use octobot_lib::metrics::{self, Metrics, ScopedGaugeCounter};
use octobot_ops::event_bus::{Event, EventBus};

const CHALLENGE_TTL: Duration = Duration::from_secs(60);
const MAX_PENDING_CHALLENGES: usize = 10_000;
const KEYS_TTL: Duration = Duration::from_secs(5 * 60);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
const MAX_STREAMS_PER_USER: usize = 10;
const MAX_STREAMS: usize = 1_000;
// Small, so a stuck client lags on the bus and gets a Reset instead of buffering here.
const STREAM_BUFFER: usize = 64;

/// Serves the event bus to clients that prove they hold an SSH key registered to their GitHub
/// login. Each stream carries only the changes octobot notified that login about.
pub struct EventsService {
    bus: Arc<EventBus>,
    authority: String,
    challenges: Challenges,
    keys: KeyCache,
    streams: Arc<StreamCounts>,
    metrics: Arc<Metrics>,
}

impl EventsService {
    pub fn new(
        bus: Arc<EventBus>,
        authority: &str,
        keys: Arc<dyn KeySource>,
        metrics: Arc<Metrics>,
    ) -> EventsService {
        EventsService {
            bus,
            authority: authority.to_string(),
            challenges: Challenges::new(),
            keys: KeyCache::new(keys),
            streams: Arc::new(StreamCounts::new()),
            metrics,
        }
    }

    async fn authenticate(&self, metadata: &MetadataMap) -> Result<String, Status> {
        let result = self.verify(metadata).await;
        if let Err(failure) = &result {
            self.metrics
                .event_stream_auth_failures
                .with_label_values(&[failure.reason()])
                .inc();
        }
        result.map_err(AuthFailure::status)
    }

    async fn verify(&self, metadata: &MetadataMap) -> Result<String, AuthFailure> {
        let login = metadata
            .get(USER_HEADER)
            .and_then(|v| v.to_str().ok())
            .ok_or(AuthFailure::Malformed("missing user"))?
            .to_lowercase();
        let nonce = metadata
            .get_bin(NONCE_HEADER)
            .and_then(|v| v.to_bytes().ok())
            .ok_or(AuthFailure::Malformed("missing nonce"))?;
        let sig = metadata
            .get_bin(SIG_HEADER)
            .and_then(|v| v.to_bytes().ok())
            .ok_or(AuthFailure::Malformed("missing signature"))?;
        let sig = SshSig::from_pem(&sig).map_err(|_| AuthFailure::Malformed("bad signature"))?;

        if !self.challenges.take(&nonce, &login) {
            return Err(AuthFailure::Nonce);
        }
        let msg = octobot_events::signed_message(&self.authority, &nonce);
        self.keys.verify(&login, &msg, &sig).await?;
        Ok(login)
    }
}

#[tonic::async_trait]
impl Events for EventsService {
    async fn challenge(
        &self,
        req: Request<ChallengeRequest>,
    ) -> Result<Response<ChallengeResponse>, Status> {
        let login = req.into_inner().username.to_lowercase();
        // The login goes into a GitHub API path.
        if login.is_empty()
            || !login
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(Status::invalid_argument("invalid username"));
        }

        let (nonce, expires) = self.challenges.issue(&login)?;
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
        let login = self.authenticate(req.metadata()).await?;
        let slot = StreamCounts::open(&self.streams, &login)?;
        let req = req.into_inner();
        info!("Opening event stream for {}", login);

        let stream = Stream {
            login,
            kinds: req.kinds.into_iter().collect(),
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
    Nonce,
    Signature,
    KeyLookup(anyhow::Error),
}

impl AuthFailure {
    fn reason(&self) -> &'static str {
        match self {
            AuthFailure::Malformed(_) => "malformed",
            AuthFailure::Nonce => "nonce",
            AuthFailure::Signature => "signature",
            AuthFailure::KeyLookup(_) => "key_lookup",
        }
    }

    fn status(self) -> Status {
        match self {
            AuthFailure::Malformed(msg) => Status::unauthenticated(msg),
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

/// Single-use nonces, each bound to the login it was issued for.
struct Challenges {
    random: SystemRandom,
    pending: Mutex<HashMap<Vec<u8>, (String, Instant)>>,
}

impl Challenges {
    fn new() -> Challenges {
        Challenges {
            random: SystemRandom::new(),
            pending: Mutex::new(HashMap::new()),
        }
    }

    fn issue(&self, login: &str) -> Result<(Vec<u8>, SystemTime), Status> {
        let mut nonce = vec![0; 32];
        self.random
            .fill(&mut nonce)
            .map_err(|_| Status::internal("could not generate nonce"))?;

        let now = Instant::now();
        let mut pending = self.pending.lock().unwrap();
        pending.retain(|_, (_, expiry)| *expiry > now);
        if pending.len() >= MAX_PENDING_CHALLENGES {
            return Err(Status::resource_exhausted("too many pending challenges"));
        }
        pending.insert(nonce.clone(), (login.to_string(), now + CHALLENGE_TTL));
        Ok((nonce, SystemTime::now() + CHALLENGE_TTL))
    }

    fn take(&self, nonce: &[u8], login: &str) -> bool {
        match self.pending.lock().unwrap().remove(nonce) {
            Some((issued_to, expiry)) => issued_to == login && expiry > Instant::now(),
            None => false,
        }
    }
}

#[async_trait]
pub trait KeySource: Send + Sync {
    /// The user's public SSH keys, in OpenSSH format.
    async fn keys(&self, login: &str) -> anyhow::Result<Vec<String>>;
}

pub struct GithubKeys(pub Arc<dyn GithubSessionFactory>);

#[async_trait]
impl KeySource for GithubKeys {
    async fn keys(&self, login: &str) -> anyhow::Result<Vec<String>> {
        self.0.get_user_keys(login).await
    }
}

/// Removing a key on GitHub revokes it here within `KEYS_TTL`.
struct KeyCache {
    source: Arc<dyn KeySource>,
    entries: Mutex<HashMap<String, CachedKeys>>,
}

struct CachedKeys {
    fetched: Instant,
    keys: Arc<Vec<PublicKey>>,
}

impl KeyCache {
    fn new(source: Arc<dyn KeySource>) -> KeyCache {
        KeyCache {
            source,
            entries: Mutex::new(HashMap::new()),
        }
    }

    async fn verify(&self, login: &str, msg: &[u8], sig: &SshSig) -> Result<(), AuthFailure> {
        let matches =
            |keys: &[PublicKey]| keys.iter().any(|k| k.verify(NAMESPACE, msg, sig).is_ok());

        let cached = self
            .entries
            .lock()
            .unwrap()
            .get(login)
            .filter(|cached| cached.fetched.elapsed() < KEYS_TTL)
            .map(|cached| cached.keys.clone());
        if cached.is_some_and(|keys| matches(&keys)) {
            return Ok(());
        }

        // Missing or stale, or the key was added on GitHub since the fetch.
        let keys = self.fetch(login).await.map_err(AuthFailure::KeyLookup)?;
        if matches(&keys) {
            Ok(())
        } else {
            Err(AuthFailure::Signature)
        }
    }

    async fn fetch(&self, login: &str) -> anyhow::Result<Arc<Vec<PublicKey>>> {
        let keys: Vec<PublicKey> = self
            .source
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
        let keys = Arc::new(keys);
        self.entries.lock().unwrap().insert(
            login.to_string(),
            CachedKeys {
                fetched: Instant::now(),
                keys: keys.clone(),
            },
        );
        Ok(keys)
    }
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
    login: String,
    /// Empty means all.
    kinds: HashSet<String>,
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
        loop {
            let body = tokio::select! {
                _ = tx.closed() => return,
                _ = heartbeat.tick() => event::Body::Heartbeat(pb::Heartbeat {}),
                received = receiver.recv() => match received {
                    Ok(event) => {
                        if !self.wants(&event) {
                            continue;
                        }
                        event::Body::Change(pb::Change {
                            repo: event.change.repo.clone(),
                            kind: event.change.kind.clone(),
                            action: event.change.action.clone(),
                            number: event.change.number,
                        })
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => event::Body::Reset(pb::Reset {}),
                    Err(broadcast::error::RecvError::Closed) => return,
                },
            };
            let kind = match body {
                event::Body::Change(_) => "change",
                event::Body::Heartbeat(_) => "heartbeat",
                event::Body::Reset(_) => "reset",
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

    fn wants(&self, event: &Event) -> bool {
        event.is_for(&self.login)
            && (self.kinds.is_empty() || self.kinds.contains(&event.change.kind))
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        info!("Closed event stream for {}", self.login);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn test_challenge_expires() {
        let challenges = Challenges::new();
        let (fresh, _) = challenges.issue("joe").unwrap();
        let (stale, _) = challenges.issue("joe").unwrap();

        tokio::time::advance(CHALLENGE_TTL - Duration::from_secs(1)).await;
        assert!(challenges.take(&fresh, "joe"));

        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(!challenges.take(&stale, "joe"));
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
