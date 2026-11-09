use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use log::warn;
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
    /// Changes may have been missed: load state from GitHub again.
    Reset,
    /// Refetch this entity from GitHub.
    Change(pb::Change),
}

/// Subscribes and stays subscribed across disconnects and server restarts. Each connect yields a
/// `Reset`, so loading initial state is the same as recovering from missed changes; updates
/// that arrive during the load are buffered.
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
            tokio::select! {
                _ = tx.closed() => return,
                result = run(&config, &signer, &tx, &mut accepted, &mut connected) => {
                    if let Err(e) = result {
                        warn!("octobot event stream: {:#}", e);
                    }
                }
            }
            if connected {
                backoff = MIN_BACKOFF;
            }
            tokio::select! {
                _ = tx.closed() => return,
                _ = tokio::time::sleep(jitter(backoff)) => {}
            }
            backoff = (backoff * 2).min(MAX_BACKOFF);
        }
    });
    ReceiverStream::new(rx)
}

async fn run(
    config: &Config,
    signer: &Arc<dyn Signer>,
    tx: &mpsc::Sender<Update>,
    accepted: &mut Option<PublicKey>,
    connected: &mut bool,
) -> Result<()> {
    let uri: Uri = config.endpoint.parse().context("parsing endpoint")?;
    let authority = crate::signed_authority(&uri)?;

    let mut endpoint = Endpoint::from(uri.clone());
    if uri.scheme_str() == Some("https") {
        endpoint = endpoint.tls_config(ClientTlsConfig::new().with_native_roots())?;
    }
    let channel = deadline("connecting", endpoint.connect()).await??;
    let mut client = EventsClient::new(channel);

    let challenge = client.challenge(ChallengeRequest {
        username: config.username.clone(),
    });
    let nonce = deadline("challenge", challenge).await??.into_inner().nonce;
    let msg = Arc::new(crate::signed_message(&authority, &nonce));

    let mut keys = blocking({
        let signer = signer.clone();
        move || signer.keys()
    })
    .await?;
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
        .await?
        .to_pem(LineEnding::LF)?;

        let req = SubscribeRequest {
            auth: Some(pb::Auth {
                username: config.username.clone(),
                nonce: nonce.clone(),
                signature: sig,
            }),
            kinds: config.kinds.iter().map(|&kind| kind as i32).collect(),
        };
        match deadline("subscribe", client.subscribe(req)).await? {
            Ok(response) => {
                *accepted = Some(key);
                stream = Some(response.into_inner());
                break;
            }
            Err(status) if status.code() == Code::Unauthenticated => continue,
            Err(status) => return Err(status.into()),
        }
    }
    let mut stream = stream.ok_or_else(|| {
        anyhow!(
            "no key was accepted for {}: is it an octobot user, with a key from the signer \
             registered on GitHub?",
            config.username
        )
    })?;
    *connected = true;

    // The stream is live only, so anything before this point may have been missed. The server
    // subscribed before answering, so a load started now misses nothing after it.
    if tx.send(Update::Reset).await.is_err() {
        return Ok(());
    }

    loop {
        let event = match tokio::time::timeout(SILENCE_TIMEOUT, stream.message()).await {
            Err(_) => return Err(anyhow!("no heartbeat for {:?}", SILENCE_TIMEOUT)),
            Ok(message) => match message? {
                Some(event) => event,
                None => return Ok(()),
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
    use tokio::net::TcpListener;

    use super::*;

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
        let _updates = subscribe(
            Config {
                endpoint: format!("http://{}", listener.local_addr().unwrap()),
                username: "joe".into(),
                kinds: vec![],
            },
            Arc::new(NeverSigns),
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
