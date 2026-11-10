use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use log::info;
use rand::Rng;
use ssh_key::{LineEnding, PublicKey};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::Code;
use tonic::codegen::http::Uri;
use tonic::transport::{ClientTlsConfig, Endpoint};

use crate::pb::events_client::EventsClient;
use crate::pb::{self, ChallengeRequest, SubscribeRequest, event};
use crate::sign::Signer;

const MIN_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(60);
// Three missed heartbeats.
const SILENCE_TIMEOUT: Duration = Duration::from_secs(90);
// Short in tests, which drive it in real time against a socket.
const RPC_TIMEOUT: Duration = if cfg!(test) {
    Duration::from_millis(200)
} else {
    Duration::from_secs(30)
};

pub struct Config {
    /// e.g. `https://octobot.example.com`
    pub endpoint: String,
    /// GitHub login
    pub username: String,
    /// Kinds to receive; empty means all.
    pub kinds: Vec<pb::Kind>,
}

#[derive(Debug, PartialEq)]
pub enum Update {
    /// Connected and subscribed. Changes may have been missed: load state from GitHub again.
    /// Changes that arrive during the load are buffered.
    Reset,
    /// Refetch this entity from GitHub.
    Change(pb::Change),
    /// Not connected: an attempt failed or a live stream ended. The client tries again after
    /// `retry_in` on its own; nothing arrives until the next `Reset`.
    Disconnected {
        cause: Cause,
        /// The failure, fit to show the user
        error: String,
        retry_in: Duration,
    },
}

/// Why the client is disconnected, by what the consumer can do about it
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cause {
    /// Something the user must fix: no key was accepted, the signer is unusable, or the
    /// endpoint or username is invalid. Retrying will not help until they do.
    Config,
    /// Octobot could not be reached or could not answer.
    Unreachable,
    /// A live stream ended: the server closed or aborted it, or it went silent.
    Ended,
}

/// Subscribes and stays subscribed across disconnects and server restarts.
///
/// Updates form one sequence: `Disconnected* Reset Change* Disconnected+ Reset Change* ...`.
/// Every failed attempt or ended stream, including before the first connect, yields exactly one
/// `Disconnected`, and every connect yields a `Reset`. So loading initial state is the same as
/// recovering from missed changes.
pub fn subscribe(config: Config, signer: Arc<dyn Signer>) -> ReceiverStream<Update> {
    let (tx, rx) = mpsc::channel(256);
    tokio::spawn(async move {
        let mut backoff = MIN_BACKOFF;
        // Tried first on reconnect, so a hardware key with a touch prompt is usually the only
        // one asked.
        let mut accepted = None;
        loop {
            // Heartbeats are not passed on, so a quiet stream would never notice that the
            // consumer is gone and would hold its server slot forever.
            let mut connected = false;
            let failure = tokio::select! {
                _ = tx.closed() => return,
                result = run(&config, &signer, &tx, &mut accepted, &mut connected) => match result {
                    Ok(()) => return,
                    Err(failure) => failure,
                },
            };
            if connected {
                backoff = MIN_BACKOFF;
            }
            let retry_in = jitter(backoff);
            let error = format!("{:#}", failure.error);
            info!("octobot event stream disconnected: {}", error);
            let update = Update::Disconnected {
                cause: failure.cause,
                error,
                retry_in,
            };
            if tx.send(update).await.is_err() {
                return;
            }
            tokio::select! {
                _ = tx.closed() => return,
                _ = tokio::time::sleep(retry_in) => {}
            }
            backoff = (backoff * 2).min(MAX_BACKOFF);
        }
    });
    ReceiverStream::new(rx)
}

struct Failure {
    cause: Cause,
    error: anyhow::Error,
}

fn config(error: impl Into<anyhow::Error>) -> Failure {
    Failure {
        cause: Cause::Config,
        error: error.into(),
    }
}

fn unreachable(error: impl Into<anyhow::Error>) -> Failure {
    Failure {
        cause: Cause::Unreachable,
        error: error.into(),
    }
}

fn ended(error: impl Into<anyhow::Error>) -> Failure {
    Failure {
        cause: Cause::Ended,
        error: error.into(),
    }
}

// A malformed username is ours to fix; anything else, such as UNAVAILABLE while octobot cannot
// reach GitHub, may pass.
fn refused(status: tonic::Status) -> Failure {
    match status.code() {
        Code::InvalidArgument => config(status),
        _ => unreachable(status),
    }
}

/// Ok only once the consumer is gone.
async fn run(
    config: &Config,
    signer: &Arc<dyn Signer>,
    tx: &mpsc::Sender<Update>,
    accepted: &mut Option<PublicKey>,
    connected: &mut bool,
) -> Result<(), Failure> {
    let uri: Uri = config
        .endpoint
        .parse()
        .context("parsing endpoint")
        .map_err(self::config)?;
    let authority = crate::signed_authority(&uri).map_err(self::config)?;

    let mut endpoint = Endpoint::from(uri.clone());
    if uri.scheme_str() == Some("https") {
        endpoint = endpoint
            .tls_config(ClientTlsConfig::new().with_native_roots())
            .map_err(self::config)?;
    }
    let channel = deadline("connecting", endpoint.connect())
        .await
        .map_err(unreachable)?
        .map_err(unreachable)?;
    let mut client = EventsClient::new(channel);

    let challenge = client.challenge(ChallengeRequest {
        username: config.username.clone(),
    });
    let nonce = deadline("challenge", challenge)
        .await
        .map_err(unreachable)?
        .map_err(refused)?
        .into_inner()
        .nonce;
    let msg = Arc::new(crate::signed_message(&authority, &nonce));

    let mut keys = blocking({
        let signer = signer.clone();
        move || signer.keys()
    })
    .await
    .map_err(self::config)?;
    if let Some(i) = keys.iter().position(|key| Some(key) == accepted.as_ref()) {
        keys.swap(0, i);
    }

    // The server refuses a wrong key the same way it refuses everything else, and the nonce
    // stays valid until a subscribe succeeds, so each key gets a turn with it.
    let mut stream = None;
    for key in keys {
        let sig = blocking({
            let (signer, key, msg) = (signer.clone(), key.clone(), msg.clone());
            move || signer.sign(&key, &msg)
        })
        .await
        .map_err(self::config)?
        .to_pem(LineEnding::LF)
        .map_err(self::config)?;

        let req = SubscribeRequest {
            auth: Some(pb::Auth {
                username: config.username.clone(),
                nonce: nonce.clone(),
                signature: sig,
            }),
            kinds: config.kinds.iter().map(|&kind| kind as i32).collect(),
        };
        match deadline("subscribe", client.subscribe(req))
            .await
            .map_err(unreachable)?
        {
            Ok(response) => {
                *accepted = Some(key);
                stream = Some(response.into_inner());
                break;
            }
            Err(status) if status.code() == Code::Unauthenticated => continue,
            Err(status) => return Err(refused(status)),
        }
    }
    let mut stream = stream.ok_or_else(|| {
        self::config(anyhow!(
            "no key was accepted for {}: is it an octobot user, with a key from the signer \
             registered on GitHub?",
            config.username
        ))
    })?;
    *connected = true;

    // The stream is live only, so anything before this point may have been missed. The server
    // subscribed before answering, so a load started now misses nothing after it.
    if tx.send(Update::Reset).await.is_err() {
        return Ok(());
    }

    loop {
        let event = match tokio::time::timeout(SILENCE_TIMEOUT, stream.message()).await {
            Err(_) => return Err(ended(anyhow!("no heartbeat for {:?}", SILENCE_TIMEOUT))),
            Ok(message) => match message.map_err(ended)? {
                Some(event) => event,
                None => return Err(ended(anyhow!("stream ended by server"))),
            },
        };

        let update = match event.body {
            Some(event::Body::Change(change)) => Update::Change(change),
            Some(event::Body::Heartbeat(_)) | None => continue,
        };
        if tx.send(update).await.is_err() {
            return Ok(());
        }
    }
}

// ssh-agent I/O blocks, and a hardware key waits for a touch.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(f).await?
}

/// Bounds a step before the stream starts, where the silence timeout does not yet apply, so a
/// server that never answers sends the client back around the retry loop.
async fn deadline<F: Future>(step: &str, fut: F) -> Result<F::Output> {
    tokio::time::timeout(RPC_TIMEOUT, fut)
        .await
        .map_err(|_| anyhow!("{} timed out after {:?}", step, RPC_TIMEOUT))
}

fn jitter(backoff: Duration) -> Duration {
    backoff.mul_f64(rand::rng().random_range(0.5..1.0))
}

#[cfg(test)]
mod tests {
    use ssh_key::rand_core::OsRng;
    use ssh_key::{Algorithm, PrivateKey};
    use tokio::net::TcpListener;
    use tokio_stream::StreamExt;
    use tokio_stream::wrappers::TcpListenerStream;
    use tonic::{Request, Response, Status};

    use super::*;
    use crate::pb::ChallengeResponse;
    use crate::pb::events_server::{Events, EventsServer};

    /// Accepts every subscription, then ends it: with `end` as the error, or cleanly.
    struct EndingServer {
        end: Option<(Code, &'static str)>,
    }

    #[tonic::async_trait]
    impl Events for EndingServer {
        async fn challenge(
            &self,
            _req: Request<ChallengeRequest>,
        ) -> Result<Response<ChallengeResponse>, Status> {
            Ok(Response::new(ChallengeResponse {
                nonce: vec![1],
                expires_unix: 0,
            }))
        }

        type SubscribeStream = tokio_stream::Iter<std::vec::IntoIter<Result<pb::Event, Status>>>;

        async fn subscribe(
            &self,
            _req: Request<SubscribeRequest>,
        ) -> Result<Response<Self::SubscribeStream>, Status> {
            let end: Vec<_> = self
                .end
                .iter()
                .map(|&(code, message)| Err(Status::new(code, message)))
                .collect();
            Ok(Response::new(tokio_stream::iter(end)))
        }
    }

    async fn start(server: EndingServer) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(
            tonic::transport::Server::builder()
                .add_service(EventsServer::new(server))
                .serve_with_incoming(TcpListenerStream::new(listener)),
        );
        endpoint
    }

    fn joe(endpoint: String) -> Config {
        Config {
            endpoint,
            username: "joe".into(),
            kinds: vec![],
        }
    }

    fn new_key() -> Arc<PrivateKey> {
        Arc::new(PrivateKey::random(&mut OsRng, Algorithm::Ed25519).unwrap())
    }

    // Guards against a hang; never expected to elapse.
    async fn next(updates: &mut ReceiverStream<Update>) -> Update {
        tokio::time::timeout(Duration::from_secs(10), updates.next())
            .await
            .expect("timed out waiting for an update")
            .expect("updates ended")
    }

    #[tokio::test]
    async fn test_reports_a_stream_the_server_ends_cleanly() {
        let endpoint = start(EndingServer { end: None }).await;
        let mut updates = subscribe(joe(endpoint), new_key());

        // Twice, since a connect that worked resets the backoff the first retry doubled.
        for _ in 0..2 {
            assert_eq!(Update::Reset, next(&mut updates).await);
            let disconnected = next(&mut updates).await;
            assert!(
                matches!(&disconnected, Update::Disconnected { cause: Cause::Ended, error, retry_in }
                    if error.contains("stream ended by server") && *retry_in <= MIN_BACKOFF),
                "{:?}",
                disconnected
            );
        }
    }

    #[tokio::test]
    async fn test_reports_a_stream_the_server_aborts() {
        let endpoint = start(EndingServer {
            end: Some((Code::Aborted, "stream fell behind")),
        })
        .await;
        let mut updates = subscribe(joe(endpoint), new_key());

        assert_eq!(Update::Reset, next(&mut updates).await);
        let disconnected = next(&mut updates).await;
        assert!(
            matches!(&disconnected, Update::Disconnected { cause: Cause::Ended, error, .. }
                if error.contains("stream fell behind")),
            "{:?}",
            disconnected
        );
    }

    struct NeverSigns;

    impl Signer for NeverSigns {
        fn keys(&self) -> Result<Vec<PublicKey>> {
            panic!("a silent server never gets as far as a challenge")
        }

        fn sign(&self, _key: &PublicKey, _msg: &[u8]) -> Result<ssh_key::SshSig> {
            panic!("a silent server never gets as far as a challenge")
        }
    }

    #[tokio::test]
    async fn test_retries_a_server_that_never_answers() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let mut updates = subscribe(joe(endpoint), Arc::new(NeverSigns));

        let disconnected = next(&mut updates).await;
        assert!(
            matches!(&disconnected, Update::Disconnected { cause: Cause::Unreachable, error, retry_in }
                if error.contains("timed out") && *retry_in > Duration::ZERO),
            "{:?}",
            disconnected
        );

        // Held open and never answered. A second connect means the first timed out.
        let mut silent = vec![];
        for _ in 0..2 {
            let accepted = tokio::time::timeout(Duration::from_secs(10), listener.accept())
                .await
                .expect("client did not retry");
            silent.push(accepted.unwrap());
        }
    }
}
