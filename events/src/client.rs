use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use log::warn;
use rand::Rng;
use ssh_key::LineEnding;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::codegen::http::Uri;
use tonic::metadata::{BinaryMetadataValue, MetadataValue};
use tonic::transport::{ClientTlsConfig, Endpoint};

use crate::pb::events_client::EventsClient;
use crate::pb::{self, ChallengeRequest, SubscribeRequest, event};
use crate::sign::Signer;

const MIN_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(60);
// Three missed heartbeats.
const SILENCE_TIMEOUT: Duration = Duration::from_secs(90);

pub struct Config {
    /// e.g. `https://octobot.example.com`
    pub endpoint: String,
    /// GitHub login
    pub username: String,
    /// GitHub event names to receive; empty means all.
    pub kinds: Vec<String>,
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
        loop {
            let mut connected = false;
            if let Err(e) = run(&config, signer.as_ref(), &tx, &mut connected).await {
                warn!("octobot event stream: {:#}", e);
            }
            if tx.is_closed() {
                return;
            }
            if connected {
                backoff = MIN_BACKOFF;
            }
            tokio::time::sleep(jitter(backoff)).await;
            backoff = (backoff * 2).min(MAX_BACKOFF);
        }
    });
    ReceiverStream::new(rx)
}

async fn run(
    config: &Config,
    signer: &dyn Signer,
    tx: &mpsc::Sender<Update>,
    connected: &mut bool,
) -> Result<()> {
    let uri: Uri = config.endpoint.parse().context("parsing endpoint")?;
    let authority = uri
        .authority()
        .ok_or_else(|| anyhow!("endpoint has no host: {}", config.endpoint))?
        .to_string();

    let mut endpoint = Endpoint::from(uri.clone());
    if uri.scheme_str() == Some("https") {
        endpoint = endpoint.tls_config(ClientTlsConfig::new().with_native_roots())?;
    }
    let mut client = EventsClient::new(endpoint.connect().await?);

    let nonce = client
        .challenge(ChallengeRequest {
            username: config.username.clone(),
        })
        .await?
        .into_inner()
        .nonce;
    let sig = signer
        .sign(&crate::signed_message(&authority, &nonce))
        .context("signing challenge")?
        .to_pem(LineEnding::LF)?;

    let mut req = tonic::Request::new(SubscribeRequest {
        kinds: config.kinds.clone(),
    });
    let metadata = req.metadata_mut();
    metadata.insert(crate::USER_HEADER, config.username.parse()?);
    metadata.insert_bin(crate::NONCE_HEADER, bin(&nonce));
    metadata.insert_bin(crate::SIG_HEADER, bin(sig.as_bytes()));
    let mut stream = client.subscribe(req).await?.into_inner();
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
            Some(event::Body::Reset(_)) => Update::Reset,
            Some(event::Body::Heartbeat(_)) | None => continue,
        };
        if tx.send(update).await.is_err() {
            return Ok(());
        }
    }
}

fn bin(bytes: &[u8]) -> BinaryMetadataValue {
    MetadataValue::from_bytes(bytes)
}

fn jitter(backoff: Duration) -> Duration {
    backoff.mul_f64(rand::rng().random_range(0.5..1.0))
}
