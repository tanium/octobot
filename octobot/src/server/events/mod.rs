use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, UNIX_EPOCH};

use async_trait::async_trait;
use log::{info, warn};
use ssh_key::{HashAlg, PublicKey, SshSig};
use tokio::sync::{broadcast, mpsc};
use tokio::time::Instant;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};

use octobot_events::pb::events_server::Events;
use octobot_events::pb::{self, ChallengeRequest, ChallengeResponse, SubscribeRequest, event};
use octobot_lib::config::Config;
use octobot_lib::github::Login;
use octobot_lib::github::api::GithubSessionFactory;
use octobot_lib::metrics::{self, Metrics, ScopedGaugeCounter};
use octobot_ops::event_bus::{Event, EventBus};

mod challenges;
mod keys;

use challenges::Challenges;
#[cfg(test)]
use keys::KEYS_REFETCH_FLOOR;
use keys::{KEYS_TTL, KeyCache};

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
const MAX_STREAMS_PER_USER: usize = 10;
const MAX_STREAMS: usize = 1_000;
// Events are tens of bytes, so this is cheap headroom for bursts. A client that lets it fill
// has stopped reading, and its stream is aborted.
const STREAM_BUFFER: usize = 256;

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
            // Not the username: it is the caller's unverified claim, and the client's own error
            // names it.
            match failure {
                AuthFailure::Malformed(detail) => info!("Refused event stream: {}", detail),
                _ => info!("Refused event stream: {}", failure.reason()),
            }
            self.metrics
                .event_stream_auth_failures
                .with_label_values(&[failure.reason()])
                .inc();
        }
        result.map_err(AuthFailure::status)
    }

    async fn verify(&self, auth: Option<&pb::Auth>) -> Result<(Login, PublicKey), AuthFailure> {
        let auth = auth.ok_or(AuthFailure::Malformed("missing auth"))?;
        let username =
            Login::parse(&auth.username).map_err(|_| AuthFailure::Malformed("bad username"))?;
        let expiry = self
            .challenges
            .check(&auth.nonce, username.as_str())
            .ok_or(AuthFailure::Nonce)?;
        let login = self.octobot_user(username.as_str())?;
        let sig = SshSig::from_pem(&auth.signature)
            .map_err(|_| AuthFailure::Malformed("bad signature"))?;

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
        let username = req.into_inner().username;
        Login::parse(&username).map_err(|_| Status::invalid_argument("invalid username"))?;
        let login = self.octobot_user(&username).map_err(AuthFailure::status)?;
        // Public on GitHub anyway; listing them lets the client sign with one GitHub knows.
        // Subscribe then usually finds them cached.
        let keys = self
            .keys
            .recent(&login)
            .await
            .map_err(|e| AuthFailure::KeyLookup(e).status())?;
        let (nonce, expires) = self.challenges.issue(login.as_str())?;
        Ok(Response::new(ChallengeResponse {
            nonce,
            expires_unix: expires.duration_since(UNIX_EPOCH).unwrap().as_secs() as i64,
            key_fingerprints: keys
                .iter()
                .map(|key| key.fingerprint(HashAlg::Sha256).to_string())
                .collect(),
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
            // Not a refusal: the challenge expired or outlived a restart, and a new one works.
            AuthFailure::Nonce => Status::failed_precondition("unknown or expired challenge"),
            AuthFailure::Signature => {
                Status::unauthenticated("signature does not match the user's GitHub keys")
            }
            AuthFailure::UserLookup(e) => {
                warn!("Error looking up octobot user: {:#}", e);
                Status::unavailable("could not look up octobot users")
            }
            AuthFailure::KeyLookup(e) => {
                warn!("Error looking up SSH keys: {:#}", e);
                Status::unavailable("could not look up the user's GitHub keys")
            }
        }
    }
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
                        let _ = tx.try_send(Err(revoked));
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
                            kind: pb::Kind::from(event.change.kind) as i32,
                            action: event.change.action.clone(),
                            number: event.change.number,
                        })
                    }
                    // Changes were dropped. Ending the stream makes the client reconnect and
                    // reload, the same as after any other disconnect.
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        self.abort();
                        let _ = tx.try_send(Err(Status::aborted("stream fell behind")));
                        return;
                    }
                    Err(broadcast::error::RecvError::Closed) => return,
                },
            };
            let kind = match body {
                event::Body::Change(_) => "change",
                event::Body::Heartbeat(_) => "heartbeat",
            };
            // Never waits on the client: while waiting, the stream could not notice it falling
            // behind or going away, and would hold its slot. A full buffer means the client
            // stopped reading, so the stream ends and the client reloads when it reconnects.
            match tx.try_send(Ok(pb::Event { body: Some(body) })) {
                Ok(()) => self
                    .metrics
                    .event_stream_messages
                    .with_label_values(&[kind])
                    .inc(),
                Err(mpsc::error::TrySendError::Full(_)) => {
                    self.abort();
                    return;
                }
                Err(mpsc::error::TrySendError::Closed(_)) => return,
            }
        }
    }

    fn abort(&self) {
        info!("Aborting lagging event stream for {}", self.login);
        self.metrics
            .event_stream_messages
            .with_label_values(&["aborted"])
            .inc();
    }

    async fn still_allowed(&self) -> anyhow::Result<bool> {
        let user = self.directory.user(self.login.as_str())?;
        Ok(user.is_some() && self.keys.has(&self.login, &self.key).await?)
    }

    fn wants(&self, event: &Event) -> bool {
        event.is_for(self.login.as_str())
            && (self.kinds.is_empty() || self.kinds.contains(&pb::Kind::from(event.change.kind)))
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        info!("Closed event stream for {}", self.login);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use ssh_key::PrivateKey;
    use ssh_key::rand_core::OsRng;

    use super::*;

    #[derive(Default)]
    pub(super) struct FakeDirectory {
        keys: Mutex<HashMap<String, Vec<String>>>,
        removed_users: Mutex<HashSet<String>>,
        pub(super) lookups: AtomicUsize,
        pub(super) failing: AtomicBool,
        pub(super) delay: Mutex<Duration>,
    }

    impl FakeDirectory {
        pub(super) fn set(&self, login: &str, keys: &[&PrivateKey]) {
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

    pub(super) fn joe() -> Login {
        Login::parse("joe").unwrap()
    }

    pub(super) fn new_key() -> PrivateKey {
        PrivateKey::random(&mut OsRng, ssh_key::Algorithm::Ed25519).unwrap()
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
    async fn test_challenge_lists_a_key_added_on_github_after_refetch_floor() {
        let directory = Arc::new(FakeDirectory::default());
        let (old, new) = (new_key(), new_key());
        directory.set("joe", &[&old]);
        let service = EventsService::new(
            Arc::new(EventBus::new(10)),
            "octobot.example.com",
            directory.clone(),
            Metrics::new(),
        )
        .unwrap();
        let listed = || async {
            let req = Request::new(ChallengeRequest {
                username: "joe".into(),
            });
            service
                .challenge(req)
                .await
                .unwrap()
                .into_inner()
                .key_fingerprints
                .len()
        };
        assert_eq!(1, listed().await);

        directory.set("joe", &[&old, &new]);
        tokio::time::advance(KEYS_REFETCH_FLOOR).await;
        assert_eq!(2, listed().await);
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

    #[tokio::test]
    async fn test_stream_ends_when_its_client_stops_reading() {
        let directory = Arc::new(FakeDirectory::default());
        let key = new_key();
        directory.set("joe", &[&key]);
        let (bus, rx) = start_stream(directory, &key);

        // Nothing reads `rx` while the stream overfills it.
        for number in 0..2 * STREAM_BUFFER as u64 {
            bus.publish(
                vec!["joe".to_string()],
                octobot_ops::event_bus::Change {
                    repo: "some-org/some-repo".into(),
                    kind: octobot_events::Kind::Push,
                    action: String::new(),
                    number: Some(number),
                },
            );
            tokio::task::yield_now().await;
        }

        // Ended without anyone reading, which frees its slot.
        assert!(rx.is_closed(), "stream still open");
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
