use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use ssh_key::rand_core::OsRng;
use ssh_key::{Algorithm, LineEnding, PrivateKey};
use tokio::net::TcpListener;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::{ReceiverStream, TcpListenerStream};
use tonic::metadata::MetadataValue;
use tonic::{Code, Status, Streaming};

use octobot::server::events::{EventsService, KeySource};
use octobot_events::client::{self, Update};
use octobot_events::pb::events_client::EventsClient;
use octobot_events::pb::events_server::EventsServer;
use octobot_events::pb::{self, ChallengeRequest, SubscribeRequest, event};
use octobot_events::sign::Signer;
use octobot_lib::metrics::Metrics;
use octobot_ops::event_bus::{Change, EventBus};

#[derive(Default)]
struct FakeKeys {
    keys: Mutex<HashMap<String, Vec<String>>>,
}

impl FakeKeys {
    fn add(&self, login: &str, key: &PrivateKey) {
        self.keys
            .lock()
            .unwrap()
            .entry(login.to_string())
            .or_default()
            .push(key.public_key().to_openssh().unwrap());
    }
}

#[async_trait]
impl KeySource for FakeKeys {
    async fn keys(&self, login: &str) -> anyhow::Result<Vec<String>> {
        Ok(self
            .keys
            .lock()
            .unwrap()
            .get(login)
            .cloned()
            .unwrap_or_default())
    }
}

struct TestServer {
    addr: SocketAddr,
    bus: Arc<EventBus>,
    keys: Arc<FakeKeys>,
}

impl TestServer {
    async fn start(lag_capacity: usize) -> TestServer {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let bus = Arc::new(EventBus::new(lag_capacity));
        let keys = Arc::new(FakeKeys::default());
        let service =
            EventsService::new(bus.clone(), &addr.to_string(), keys.clone(), Metrics::new());
        tokio::spawn(
            tonic::transport::Server::builder()
                .add_service(EventsServer::new(service))
                .serve_with_incoming(TcpListenerStream::new(listener)),
        );
        TestServer { addr, bus, keys }
    }

    fn endpoint(&self) -> String {
        format!("http://{}", self.addr)
    }

    fn subscribe(&self, login: &str, kinds: &[&str], key: PrivateKey) -> ReceiverStream<Update> {
        client::subscribe(
            client::Config {
                endpoint: self.endpoint(),
                username: login.to_string(),
                kinds: kinds.iter().map(|k| k.to_string()).collect(),
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
        let sig = Signer::sign(key, &octobot_events::signed_message(authority, nonce))
            .unwrap()
            .to_pem(LineEnding::LF)
            .unwrap();
        let mut req = tonic::Request::new(SubscribeRequest { kinds: vec![] });
        let metadata = req.metadata_mut();
        metadata.insert(octobot_events::USER_HEADER, login.parse().unwrap());
        metadata.insert_bin(
            octobot_events::NONCE_HEADER,
            MetadataValue::from_bytes(nonce),
        );
        metadata.insert_bin(
            octobot_events::SIG_HEADER,
            MetadataValue::from_bytes(sig.as_bytes()),
        );
        self.raw_client()
            .await
            .subscribe(req)
            .await
            .map(|r| r.into_inner())
    }

    fn publish(&self, recipients: &[&str], kind: &str, number: u64) {
        self.bus.publish(
            recipients.iter().map(|r| r.to_string()),
            change(kind, number),
        );
    }
}

fn new_key() -> PrivateKey {
    PrivateKey::random(&mut OsRng, Algorithm::Ed25519).unwrap()
}

fn change(kind: &str, number: u64) -> Change {
    Change {
        repo: "some-org/some-repo".into(),
        kind: kind.into(),
        action: "opened".into(),
        number,
    }
}

fn pb_change(kind: &str, number: u64) -> pb::Change {
    pb::Change {
        repo: "some-org/some-repo".into(),
        kind: kind.into(),
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
    server.keys.add("joe", &key);

    let mut updates = server.subscribe("Joe", &[], key);
    assert_eq!(Update::Reset, next(&mut updates).await);

    server.publish(&["bob"], "pull_request", 1);
    server.publish(&["bob", "joe"], "pull_request", 2);
    assert_eq!(
        Update::Change(pb_change("pull_request", 2)),
        next(&mut updates).await
    );
}

#[tokio::test]
async fn test_filters_kinds() {
    let server = TestServer::start(10).await;
    let key = new_key();
    server.keys.add("joe", &key);

    let mut updates = server.subscribe("joe", &["push"], key);
    assert_eq!(Update::Reset, next(&mut updates).await);

    server.publish(&["joe"], "pull_request", 1);
    server.publish(&["joe"], "push", 2);
    assert_eq!(
        Update::Change(pb_change("push", 2)),
        next(&mut updates).await
    );
}

// The test runtime is single threaded, so the stream task cannot drain the bus between
// publishes.
#[tokio::test]
async fn test_lagging_stream_resets() {
    let server = TestServer::start(1).await;
    let key = new_key();
    server.keys.add("joe", &key);

    let nonce = server.challenge("joe").await;
    let mut stream = server
        .raw_subscribe("joe", &nonce, &key, &server.addr.to_string())
        .await
        .unwrap();

    server.publish(&["joe"], "push", 1);
    server.publish(&["joe"], "push", 2);
    server.publish(&["joe"], "push", 3);

    let body = |e: Result<pb::Event, Status>| e.unwrap().body.unwrap();
    assert_eq!(
        event::Body::Reset(pb::Reset {}),
        body(next(&mut stream).await)
    );
    assert_eq!(
        event::Body::Change(pb_change("push", 3)),
        body(next(&mut stream).await)
    );
}

#[tokio::test]
async fn test_accepts_signature_from_registered_key() {
    let server = TestServer::start(10).await;
    let key = new_key();
    server.keys.add("joe", &key);

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
    server.keys.add("joe", &new_key());

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
    server.keys.add("joe", &key);

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
    server.keys.add("joe", &key);

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
    server.keys.add("joe", &key);

    let nonce = server.challenge("joe").await;
    let err = server
        .raw_subscribe("joe", &nonce, &key, "other.example.com")
        .await
        .unwrap_err();
    assert_eq!(Code::Unauthenticated, err.code());
}

#[tokio::test]
async fn test_accepts_key_added_after_keys_were_cached() {
    let server = TestServer::start(10).await;
    let old_key = new_key();
    server.keys.add("joe", &old_key);
    let authority = server.addr.to_string();

    let nonce = server.challenge("joe").await;
    assert!(
        server
            .raw_subscribe("joe", &nonce, &old_key, &authority)
            .await
            .is_ok()
    );

    let new_key = new_key();
    server.keys.add("joe", &new_key);
    let nonce = server.challenge("joe").await;
    assert!(
        server
            .raw_subscribe("joe", &nonce, &new_key, &authority)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn test_rejects_invalid_username() {
    let server = TestServer::start(10).await;
    let err = server
        .raw_client()
        .await
        .challenge(ChallengeRequest {
            username: "../orgs/x".into(),
        })
        .await
        .unwrap_err();
    assert_eq!(Code::InvalidArgument, err.code());
}
