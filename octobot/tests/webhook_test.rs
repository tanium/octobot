use std::sync::Arc;

use anyhow::anyhow;
use async_trait::async_trait;
use hyper::body::Bytes;
use hyper::{HeaderMap, StatusCode};
use ring::hmac;
use tempfile::{TempDir, tempdir};

use octobot::server::github_handler::GithubHandlerState;
use octobot_lib::config::Config;
use octobot_lib::config_db::ConfigDatabase;
use octobot_lib::errors::Result;
use octobot_lib::github::api::{GithubSession, GithubSessionFactory};
use octobot_lib::github::{HookBody, Login, Repo};
use octobot_lib::metrics::Metrics;
use octobot_ops::event_bus::EventBus;
use octobot_ops::slack::Slack;
use octobot_ops::webhook_db::WebhookDatabase;

const SECRET: &str = "the-secret";

struct FakeGithub {
    sessions_fail: bool,
}

#[async_trait]
impl GithubSessionFactory for FakeGithub {
    async fn new_service_session(&self) -> Result<GithubSession> {
        unimplemented!()
    }

    async fn new_session(&self, _owner: &str, _repo: &str) -> Result<GithubSession> {
        if self.sessions_fail {
            return Err(anyhow!("no session"));
        }
        GithubSession::new("github.com", "octobot", "token", None, None)
    }

    async fn get_token_org(&self, _org: &str) -> Result<String> {
        unimplemented!()
    }

    async fn get_token_repo(&self, _owner: &str, _repo: &str) -> Result<String> {
        unimplemented!()
    }

    async fn get_user_keys(&self, _login: &Login) -> Result<Vec<String>> {
        unimplemented!()
    }

    fn bot_name(&self) -> String {
        "octobot".into()
    }
}

struct WebhookTest {
    // The handler state owns a jobs runtime, which can't be dropped inside an async context.
    // Tests drive it from their own runtime.
    runtime: tokio::runtime::Runtime,
    state: GithubHandlerState,
    metrics: Arc<Metrics>,
    _dir: TempDir,
}

fn new_test(sessions_fail: bool) -> WebhookTest {
    let dir = tempdir().unwrap();
    let path = |name: &str| dir.path().join(name).to_string_lossy().into_owned();

    let mut config = Config::new(ConfigDatabase::new(&path("config.sqlite3")).unwrap());
    config.github.webhook_secret = SECRET.into();
    let metrics = Metrics::new();
    let state = GithubHandlerState::new(
        Arc::new(config),
        Arc::new(FakeGithub { sessions_fail }),
        None,
        Arc::new(Slack::new(
            "token".into(),
            path("slack.sqlite3"),
            metrics.clone(),
        )),
        Arc::new(WebhookDatabase::new(&path("webhooks.sqlite3")).unwrap()),
        Arc::new(EventBus::new(10)),
        metrics.clone(),
    );
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    WebhookTest {
        runtime,
        state,
        metrics,
        _dir: dir,
    }
}

fn with_repo() -> Vec<u8> {
    let mut body = HookBody::new();
    body.repository = Some(Repo::parse("https://github.com/some-user/some-repo").unwrap());
    serde_json::to_vec(&body).unwrap()
}

fn sign(body: &[u8]) -> String {
    let key = hmac::Key::new(hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY, SECRET.as_bytes());
    format!("sha1={}", hex::encode(hmac::sign(&key, body).as_ref()))
}

impl WebhookTest {
    fn send(&self, delivery: &str, event: &str, body: &[u8], signature: &str) -> StatusCode {
        let mut headers = HeaderMap::new();
        headers.insert("x-github-delivery", delivery.parse().unwrap());
        headers.insert("x-github-event", event.parse().unwrap());
        headers.insert("x-hub-signature", signature.parse().unwrap());
        let body = Bytes::copy_from_slice(body);
        self.runtime
            .block_on(self.state.receive(&headers, Ok(body)))
            .status()
    }

    fn send_signed(&self, delivery: &str, event: &str, body: &[u8]) -> StatusCode {
        self.send(delivery, event, body, &sign(body))
    }

    fn count(&self, event: &str, outcome: &str) -> u64 {
        self.metrics
            .webhook_events
            .with_label_values(&[event, outcome])
            .get()
    }
}

#[test]
fn test_handled_event_is_counted() {
    let test = new_test(false);

    assert_eq!(StatusCode::OK, test.send_signed("1", "ping", &with_repo()));
    assert_eq!(1, test.count("ping", "handled"));
}

#[test]
fn test_unhandled_event_is_ignored() {
    let test = new_test(false);

    assert_eq!(StatusCode::OK, test.send_signed("1", "check_run", &with_repo()));
    assert_eq!(1, test.count("check_run", "ignored"));
}

#[test]
fn test_event_without_repository_is_ignored() {
    let test = new_test(false);
    let body = serde_json::to_vec(&HookBody::new()).unwrap();

    assert_eq!(StatusCode::OK, test.send_signed("1", "installation", &body));
    assert_eq!(1, test.count("installation", "ignored"));
}

#[test]
fn test_duplicate_delivery_is_rejected_without_event() {
    let test = new_test(false);

    test.send_signed("1", "ping", &with_repo());
    let status = test.send_signed("1", "ping", &with_repo());

    assert_eq!(StatusCode::BAD_REQUEST, status);
    assert_eq!(1, test.count("", "rejected"));
}

#[test]
fn test_bad_signature_is_rejected_without_event() {
    let test = new_test(false);

    let status = test.send("1", "made-up-event", &with_repo(), "sha1=00");

    assert_eq!(StatusCode::FORBIDDEN, status);
    assert_eq!(1, test.count("", "rejected"));
}

#[test]
fn test_bad_json_is_rejected() {
    let test = new_test(false);

    let status = test.send_signed("1", "ping", b"not json");

    assert_eq!(StatusCode::BAD_REQUEST, status);
    assert_eq!(1, test.count("ping", "rejected"));
}

#[test]
fn test_session_failure_is_rejected() {
    let test = new_test(true);

    let status = test.send_signed("1", "ping", &with_repo());

    assert_eq!(StatusCode::BAD_REQUEST, status);
    assert_eq!(1, test.count("ping", "rejected"));
}
