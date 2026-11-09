use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use ssh_key::rand_core::OsRng;
use ssh_key::{Algorithm, HashAlg, LineEnding, PrivateKey};
use tokio::net::TcpListener;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::{ReceiverStream, TcpListenerStream};
use tonic::{Code, Status, Streaming};

use octobot::server::events::{Directory, EventsService};
use octobot_events::client::{self, Update};
use octobot_events::pb::events_client::EventsClient;
use octobot_events::pb::events_server::EventsServer;
use octobot_events::pb::{self, ChallengeRequest, SubscribeRequest};
use octobot_lib::github::Login;
use octobot_lib::metrics::Metrics;
use octobot_ops::event_bus::{Change, EventBus, Kind};

/// Octobot users and their GitHub keys. A user is anyone given a key.
#[derive(Default)]
struct FakeDirectory {
    keys: Mutex<HashMap<String, Vec<String>>>,
    lookups: AtomicUsize,
}

impl FakeDirectory {
    fn add(&self, login: &str, key: &PrivateKey) {
        self.keys
            .lock()
            .unwrap()
            .entry(login.to_string())
            .or_default()
            .push(key.public_key().to_openssh().unwrap());
    }

    fn remove(&self, login: &str) {
        self.keys.lock().unwrap().remove(login);
    }

    fn lookups(&self) -> usize {
        self.lookups.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl Directory for FakeDirectory {
    fn user(&self, username: &str) -> anyhow::Result<Option<Login>> {
        // Like the users table, matches case-insensitively before parsing what it stores.
        let name = username.to_lowercase();
        let is_user = self.keys.lock().unwrap().contains_key(&name);
        is_user.then(|| Login::parse(&name)).transpose()
    }

    async fn keys(&self, login: &Login) -> anyhow::Result<Vec<String>> {
        self.lookups.fetch_add(1, Ordering::SeqCst);
        Ok(self
            .keys
            .lock()
            .unwrap()
            .get(login.as_str())
            .cloned()
            .unwrap_or_default())
    }
}

struct TestServer {
    addr: SocketAddr,
    bus: Arc<EventBus>,
    directory: Arc<FakeDirectory>,
}

impl TestServer {
    async fn start(lag_capacity: usize) -> TestServer {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let bus = Arc::new(EventBus::new(lag_capacity));
        let directory = Arc::new(FakeDirectory::default());
        let service = EventsService::new(
            bus.clone(),
            &addr.to_string(),
            directory.clone(),
            Metrics::new(),
        )
        .unwrap();
        tokio::spawn(
            tonic::transport::Server::builder()
                .add_service(EventsServer::new(service))
                .serve_with_incoming(TcpListenerStream::new(listener)),
        );
        TestServer {
            addr,
            bus,
            directory,
        }
    }

    fn endpoint(&self) -> String {
        format!("http://{}", self.addr)
    }

    fn subscribe(
        &self,
        login: &str,
        kinds: &[pb::Kind],
        key: PrivateKey,
    ) -> ReceiverStream<Update> {
        client::subscribe(
            client::Config {
                endpoint: self.endpoint(),
                username: login.to_string(),
                kinds: kinds.to_vec(),
            },
            Arc::new(key),
        )
    }

    async fn raw_client(&self) -> EventsClient<tonic::transport::Channel> {
        EventsClient::connect(self.endpoint()).await.unwrap()
    }

    async fn challenge(&self, login: &str) -> Vec<u8> {
        self.raw_client()
            .await
            .challenge(ChallengeRequest {
                username: login.to_string(),
            })
            .await
            .unwrap()
            .into_inner()
            .nonce
    }

    /// Subscribes with `nonce` signed by `key` for `authority`.
    async fn raw_subscribe(
        &self,
        login: &str,
        nonce: &[u8],
        key: &PrivateKey,
        authority: &str,
    ) -> Result<Streaming<pb::Event>, Status> {
        // Signs directly, since some tests sign with keys GitHub does not have.
        let msg = octobot_events::signed_message(authority, nonce);
        let sig = key
            .sign(octobot_events::NAMESPACE, HashAlg::Sha512, &msg)
            .unwrap()
            .to_pem(LineEnding::LF)
            .unwrap();
        let req = SubscribeRequest {
            auth: Some(pb::Auth {
                username: login.to_string(),
                nonce: nonce.to_vec(),
                signature: sig,
            }),
            kinds: vec![],
        };
        self.raw_client()
            .await
            .subscribe(req)
            .await
            .map(|r| r.into_inner())
    }

    fn publish(&self, recipients: &[&str], kind: Kind, number: u64) {
        self.bus.publish(
            recipients.iter().map(|r| r.to_string()),
            change(kind, number),
        );
    }
}

fn new_key() -> PrivateKey {
    PrivateKey::random(&mut OsRng, Algorithm::Ed25519).unwrap()
}

fn change(kind: Kind, number: u64) -> Change {
    Change {
        repo: "some-org/some-repo".into(),
        kind,
        action: "opened".into(),
        number,
    }
}

fn pb_change(kind: pb::Kind, number: u64) -> pb::Change {
    pb::Change {
        repo: "some-org/some-repo".into(),
        kind: kind as i32,
        action: "opened".into(),
        number,
    }
}

// Guards against a hang; never expected to elapse.
async fn next<S: tokio_stream::Stream + Unpin>(stream: &mut S) -> S::Item {
    tokio::time::timeout(Duration::from_secs(10), stream.next())
        .await
        .expect("timed out waiting for stream")
        .expect("stream ended")
}

#[tokio::test]
async fn test_streams_changes_for_the_user() {
    let server = TestServer::start(10).await;
    let key = new_key();
    server.directory.add("joe", &key);

    let mut updates = server.subscribe("Joe", &[], key);
    assert_eq!(Update::Reset, next(&mut updates).await);

    server.publish(&["bob"], Kind::PullRequest, 1);
    server.publish(&["bob", "joe"], Kind::PullRequest, 2);
    assert_eq!(
        Update::Change(pb_change(pb::Kind::PullRequest, 2)),
        next(&mut updates).await
    );
}

#[tokio::test]
async fn test_filters_kinds() {
    let server = TestServer::start(10).await;
    let key = new_key();
    server.directory.add("joe", &key);

    let mut updates = server.subscribe("joe", &[pb::Kind::Push], key);
    assert_eq!(Update::Reset, next(&mut updates).await);

    server.publish(&["joe"], Kind::PullRequest, 1);
    server.publish(&["joe"], Kind::Push, 2);
    assert_eq!(
        Update::Change(pb_change(pb::Kind::Push, 2)),
        next(&mut updates).await
    );
}

// The test runtime is single threaded, so the stream task cannot drain the bus between
// publishes.
#[tokio::test]
async fn test_lagging_stream_is_aborted() {
    let server = TestServer::start(1).await;
    let key = new_key();
    server.directory.add("joe", &key);

    let nonce = server.challenge("joe").await;
    let mut stream = server
        .raw_subscribe("joe", &nonce, &key, &server.addr.to_string())
        .await
        .unwrap();

    server.publish(&["joe"], Kind::Push, 1);
    server.publish(&["joe"], Kind::Push, 2);
    server.publish(&["joe"], Kind::Push, 3);

    let status = next(&mut stream).await.unwrap_err();
    assert_eq!(Code::Aborted, status.code());
    assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn test_client_reconnects_and_resets_after_abort() {
    let server = TestServer::start(1).await;
    let key = new_key();
    server.directory.add("joe", &key);

    let mut updates = server.subscribe("joe", &[], key);
    assert_eq!(Update::Reset, next(&mut updates).await);

    server.publish(&["joe"], Kind::Push, 1);
    server.publish(&["joe"], Kind::Push, 2);
    server.publish(&["joe"], Kind::Push, 3);
    assert_eq!(Update::Reset, next(&mut updates).await);
}

#[tokio::test]
async fn test_accepts_signature_from_registered_key() {
    let server = TestServer::start(10).await;
    let key = new_key();
    server.directory.add("joe", &key);

    let nonce = server.challenge("joe").await;
    let authority = server.addr.to_string();
    assert!(
        server
            .raw_subscribe("joe", &nonce, &key, &authority)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn test_rejects_unregistered_key() {
    let server = TestServer::start(10).await;
    server.directory.add("joe", &new_key());

    let nonce = server.challenge("joe").await;
    let authority = server.addr.to_string();
    let err = server
        .raw_subscribe("joe", &nonce, &new_key(), &authority)
        .await
        .unwrap_err();
    assert_eq!(Code::Unauthenticated, err.code());
}

#[tokio::test]
async fn test_rejects_reused_nonce() {
    let server = TestServer::start(10).await;
    let key = new_key();
    server.directory.add("joe", &key);

    let nonce = server.challenge("joe").await;
    let authority = server.addr.to_string();
    assert!(
        server
            .raw_subscribe("joe", &nonce, &key, &authority)
            .await
            .is_ok()
    );
    let err = server
        .raw_subscribe("joe", &nonce, &key, &authority)
        .await
        .unwrap_err();
    assert_eq!(Code::Unauthenticated, err.code());
}

#[tokio::test]
async fn test_rejects_nonce_issued_to_another_user() {
    let server = TestServer::start(10).await;
    let key = new_key();
    server.directory.add("joe", &key);
    server.directory.add("bob", &new_key());

    let nonce = server.challenge("bob").await;
    let authority = server.addr.to_string();
    let err = server
        .raw_subscribe("joe", &nonce, &key, &authority)
        .await
        .unwrap_err();
    assert_eq!(Code::Unauthenticated, err.code());
}

#[tokio::test]
async fn test_rejects_signature_for_another_server() {
    let server = TestServer::start(10).await;
    let key = new_key();
    server.directory.add("joe", &key);

    let nonce = server.challenge("joe").await;
    let err = server
        .raw_subscribe("joe", &nonce, &key, "other.example.com")
        .await
        .unwrap_err();
    assert_eq!(Code::Unauthenticated, err.code());
}

#[tokio::test]
async fn test_refuses_subscribe_for_user_removed_after_challenge() {
    let server = TestServer::start(10).await;
    let key = new_key();
    server.directory.add("joe", &key);

    let nonce = server.challenge("joe").await;
    server.directory.remove("joe");
    let err = server
        .raw_subscribe("joe", &nonce, &key, &server.addr.to_string())
        .await
        .unwrap_err();
    assert_eq!(Code::PermissionDenied, err.code());
}

#[tokio::test]
async fn test_challenge_lists_the_users_key_fingerprints() {
    let server = TestServer::start(10).await;
    let (first, second) = (new_key(), new_key());
    server.directory.add("joe", &first);
    server.directory.add("joe", &second);

    let response = server
        .raw_client()
        .await
        .challenge(ChallengeRequest {
            username: "joe".into(),
        })
        .await
        .unwrap()
        .into_inner();
    let fingerprint = |key: &PrivateKey| key.public_key().fingerprint(HashAlg::Sha256).to_string();
    assert_eq!(
        vec![fingerprint(&first), fingerprint(&second)],
        response.key_fingerprints
    );
}

#[tokio::test]
async fn test_refuses_challenge_for_non_user() {
    let server = TestServer::start(10).await;
    let err = server
        .raw_client()
        .await
        .challenge(ChallengeRequest {
            username: "stranger".into(),
        })
        .await
        .unwrap_err();
    assert_eq!(Code::PermissionDenied, err.code());
    assert_eq!(0, server.directory.lookups());
}

#[tokio::test]
async fn test_dropped_subscriptions_release_their_streams() {
    let server = TestServer::start(10).await;
    let key = new_key();
    server.directory.add("joe", &key);

    // More than the per-user stream limit, one after another.
    for _ in 0..12 {
        let mut updates = server.subscribe("joe", &[], key.clone());
        assert_eq!(Update::Reset, next(&mut updates).await);
    }
}

#[tokio::test]
async fn test_rejects_subscribe_without_auth() {
    let server = TestServer::start(10).await;
    let err = server
        .raw_client()
        .await
        .subscribe(SubscribeRequest {
            auth: None,
            kinds: vec![],
        })
        .await
        .unwrap_err();
    assert_eq!(Code::Unauthenticated, err.code());
}
