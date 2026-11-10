use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::{TokioIo, TokioTimer};
use hyper_util::server::graceful::GracefulShutdown;
use log::{debug, error, info, warn};
use octobot_ops::event_bus::EventBus;
use octobot_ops::webhook_db::WebhookDatabase;
use tokio::net::TcpListener;
use tokio::signal::unix::{SignalKind, signal};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::http_util::Body;
use crate::runtime;
use crate::server::events::{self, EventsService, GithubDirectory};
use crate::server::github_handler::GithubHandlerState;
use crate::server::octobot_service::OctobotService;
use crate::server::sessions::Sessions;
use octobot_lib::config::Config;
use octobot_lib::github;
use octobot_lib::jira;
use octobot_lib::jira::api::JiraSession;
use octobot_lib::metrics;

use octobot_lib::github::api::Session;

// Kept under the container's stop timeout so octobot can log what it
// abandoned before it is killed.
const SHUTDOWN_DEADLINE: Duration = Duration::from_secs(25);

// How far a stream can fall behind before it is aborted.
const EVENT_LAG_CAPACITY: usize = 1_000;

pub fn start(config: Config) {
    let num_http_threads = config.main.num_http_threads.unwrap_or(20);
    let metrics = metrics::Metrics::new();

    runtime::run(num_http_threads, metrics.clone(), async move {
        run_server(config, metrics).await
    });
}

async fn run_server(config: Config, metrics: Arc<metrics::Metrics>) {
    // Octobot is PID 1 in its container, where signals without a handler are ignored rather
    // than fatal. Registering before startup means a stop is never ignored.
    let mut sigterm = signal(SignalKind::terminate()).expect("SIGTERM handler");
    let mut sigint = signal(SignalKind::interrupt()).expect("SIGINT handler");
    let stopping = CancellationToken::new();
    tokio::spawn({
        let stopping = stopping.clone();
        async move {
            tokio::select! {
                _ = sigterm.recv() => info!("Received SIGTERM"),
                _ = sigint.recv() => info!("Received SIGINT"),
            }
            stopping.cancel();
        }
    });

    let config = Arc::new(config);

    let slack_api = Arc::new(octobot_ops::slack::Slack::new(
        config.slack.bot_token.clone(),
        config.slack_db_path(),
        metrics.clone(),
    ));

    {
        let slack = slack_api.clone();
        let config = config.clone();
        tokio::spawn(async move {
            let slack = slack.clone();
            let config = config.clone();
            octobot_ops::migrate_slack::migrate_slack_id(&config, &slack).await;
        });
    }

    // Nothing is open yet, so a stop here returns at once instead of waiting on GitHub or Jira.
    let (github_api, jira_api) = tokio::select! {
        apis = connect(&config, &metrics) => apis,
        () = stopping.cancelled() => {
            info!("Stopped during startup");
            return;
        }
    };

    let webhook_db = Arc::new(WebhookDatabase::new(&config.webhook_db_path()).expect("webhook db"));
    let latest_webhook_guid = webhook_db
        .get_latest_guid()
        .expect("failed to lookup latest guid");

    let http_addr: SocketAddr = match config.main.listen_addr {
        Some(ref addr_and_port) => addr_and_port.parse().unwrap(),
        None => "0.0.0.0:3000".parse().unwrap(),
    };

    let event_bus = Arc::new(EventBus::new(EVENT_LAG_CAPACITY));
    let events_served = if let Some(ref events_config) = config.events {
        let service = EventsService::new(
            event_bus.clone(),
            &events_config.authority,
            Arc::new(GithubDirectory {
                config: config.clone(),
                github: github_api.clone(),
            }),
            stopping.clone(),
            metrics.clone(),
        )
        .expect("events authority");
        let addr = &events_config.listen_addr;
        let listener = match TcpListener::bind(addr).await {
            Ok(l) => l,
            Err(e) => panic!("Error binding {}: {}", addr, e),
        };
        info!("Listening (events gRPC) on {}", addr);
        let stopping = stopping.clone();
        Some(tokio::spawn(async move {
            if let Err(e) = events::serve(listener, service, stopping).await {
                error!("Events server error: {}", e);
            }
        }))
    } else {
        None
    };

    let ui_sessions = Arc::new(Sessions::new());
    let github_handler_state = Arc::new(GithubHandlerState::new(
        config.clone(),
        github_api.clone(),
        jira_api.clone(),
        slack_api.clone(),
        webhook_db.clone(),
        event_bus,
        metrics.clone(),
    ));
    let octobot = OctobotService::new(
        config.clone(),
        ui_sessions.clone(),
        github_handler_state.clone(),
        jira_api.clone(),
        slack_api.clone(),
        metrics.clone(),
    );

    let cleanup = tokio::spawn({
        let octobot = octobot.clone();
        async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(600));
            loop {
                interval.tick().await;
                octobot.clean();
            }
        }
    });

    let listener = match TcpListener::bind(http_addr).await {
        Ok(l) => l,
        Err(e) => panic!("Error binding {}: {}", http_addr, e),
    };
    info!("Listening (HTTP) on {}", http_addr);

    let webhook_redeliver = tokio::spawn(async move {
        let webhook_db = webhook_db.clone();

        // Wait some time for service to startup before redelivering
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;

        if let Some(guid) = latest_webhook_guid {
            log::info!("Starting webhook redelivery");

            let session = match github_api.new_service_session().await {
                Ok(s) => s,
                Err(e) => {
                    log::error!("Failed to get session to redeliver webhooks: {}", e);
                    return;
                }
            };

            let max_count = 10_000;
            let webhooks = match session.get_webhook_deliveries_since(&guid, max_count).await {
                Ok(v) => v,
                Err(e) => {
                    log::error!("Failed to lookup webhook deliveries: {}", e);
                    return;
                }
            };

            for d in webhooks {
                if d.status_code != 200 && d.status_code != 400 {
                    if webhook_db.has_guid(&d.guid) {
                        log::debug!(
                            "Skipping webhook redelivery guid {} -- {}",
                            d.guid,
                            d.status_code
                        );
                    } else {
                        log::info!(
                            "Redelivering webhook guid {} due to HTTP {}",
                            d.guid,
                            d.status_code
                        );
                        if let Err(e) = session.redeliver_webhook(d.id).await {
                            log::error!("Failed to redeliver webhook guid: {}", e);
                        }
                    }
                }
            }

            log::info!("Finished webhook redelivery");
        } else {
            log::info!("No recent webhook delivery to search for");
        }
    });

    let connections = serve(
        listener,
        move |req| octobot.clone().call(req),
        metrics,
        stopping.cancelled_owned(),
    )
    .await;

    info!("Shutting down: finishing open requests and background jobs");
    let deadline = Instant::now() + SHUTDOWN_DEADLINE;
    cleanup.abort();
    // Deliveries not yet redelivered are skipped: the next start searches only from the newest
    // webhook recorded.
    webhook_redeliver.abort();

    if tokio::time::timeout_at(deadline, connections.shutdown())
        .await
        .is_err()
    {
        warn!("Requests still open at the shutdown deadline");
    }
    if let Some(served) = events_served
        && tokio::time::timeout_at(deadline, served).await.is_err()
    {
        warn!("Event streams still open at the shutdown deadline");
    }
    let abandoned = github_handler_state
        .shutdown(deadline.saturating_duration_since(Instant::now()))
        .await;
    if abandoned > 0 {
        warn!(
            "Abandoning {} background jobs still running at the shutdown deadline",
            abandoned
        );
    }
    info!("Shutdown complete");

    // Exit rather than return: dropping the jobs runtime from inside this runtime would panic,
    // and nothing left needs cleaning up.
    std::process::exit(0);
}

async fn connect(
    config: &Config,
    metrics: &Arc<metrics::Metrics>,
) -> (
    Arc<dyn github::api::GithubSessionFactory>,
    Option<Arc<dyn jira::api::Session>>,
) {
    let github_api: Arc<dyn github::api::GithubSessionFactory> = if config.github.app_id.is_some() {
        match github::api::GithubApp::new(
            &config.github.host,
            config.github.app_id.expect("expected an app_id"),
            &config.github.app_key().expect("expected an app_key"),
            Some(metrics.clone()),
        )
        .await
        {
            Ok(s) => Arc::new(s),
            Err(e) => panic!("Error initiating github session: {}", e),
        }
    } else {
        match github::api::GithubOauthApp::new(
            &config.github.host,
            config
                .github
                .api_token
                .as_ref()
                .expect("expected an api_token"),
            Some(metrics.clone()),
        )
        .await
        {
            Ok(s) => Arc::new(s),
            Err(e) => panic!("Error initiating github session: {}", e),
        }
    };

    let jira_api: Option<Arc<dyn jira::api::Session>> = if let Some(ref jira_config) = config.jira {
        match JiraSession::new(jira_config, Some(metrics.clone())).await {
            Ok(s) => Some(Arc::new(s)),
            Err(e) => panic!("Error initiating jira session: {}", e),
        }
    } else {
        None
    };

    (github_api, jira_api)
}

/// Serves until `stop` completes, then stops accepting connections. The returned
/// `GracefulShutdown` finishes the requests still open.
async fn serve<F, Fut>(
    listener: TcpListener,
    handle: F,
    metrics: Arc<metrics::Metrics>,
    stop: impl Future<Output = ()>,
) -> GracefulShutdown
where
    F: Fn(Request<Incoming>) -> Fut + Clone + Send + 'static,
    Fut: Future<Output = Result<Response<Body>, hyper::Error>> + Send + 'static,
{
    let connections = GracefulShutdown::new();
    tokio::pin!(stop);
    loop {
        let stream = tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => stream,
                Err(e) => {
                    // Typically file descriptor exhaustion; back off instead of spinning.
                    error!("Error accepting connection: {}", e);
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    continue;
                }
            },
            () = &mut stop => return connections,
        };

        let scoped_count = metrics::scoped_inc(&metrics.current_connection_count);
        let conn = http1::Builder::new()
            .timer(TokioTimer::new())
            .serve_connection(TokioIo::new(stream), service_fn(handle.clone()));
        let conn = connections.watch(conn);
        tokio::spawn(async move {
            let _scoped_count = scoped_count;
            if let Err(e) = conn.await {
                debug!("Connection error: {}", e);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::pin::Pin;
    use std::sync::Mutex;
    use std::task::{Context, Waker};

    use hyper::StatusCode;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;
    use tokio::sync::oneshot;

    use crate::http_util;

    fn is_pending<F: Future>(fut: Pin<&mut F>) -> bool {
        fut.poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    }

    #[tokio::test]
    async fn test_stop_finishes_open_request_and_refuses_new_ones() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let (entered_tx, entered_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel::<()>();
        let gate = Arc::new(Mutex::new(Some((entered_tx, release_rx))));
        let handle = move |_req| {
            let (entered, release) = gate.lock().unwrap().take().unwrap();
            async move {
                entered.send(()).unwrap();
                release.await.unwrap();
                Ok(http_util::new_msg_resp(StatusCode::OK, "done"))
            }
        };

        let (stop_tx, stop_rx) = oneshot::channel::<()>();
        let server = tokio::spawn(serve(listener, handle, metrics::Metrics::new(), async {
            stop_rx.await.unwrap();
        }));

        let mut client = TcpStream::connect(addr).await.unwrap();
        client
            .write_all(b"GET / HTTP/1.1\r\nhost: octobot\r\n\r\n")
            .await
            .unwrap();
        entered_rx.await.unwrap();

        stop_tx.send(()).unwrap();
        let connections = server.await.unwrap();
        assert!(TcpStream::connect(addr).await.is_err());

        let mut finished = Box::pin(connections.shutdown());
        assert!(is_pending(finished.as_mut()));

        release_tx.send(()).unwrap();
        let mut resp = String::new();
        client.read_to_string(&mut resp).await.unwrap();
        assert!(resp.starts_with("HTTP/1.1 200 OK"), "{}", resp);
        assert!(resp.ends_with("done"), "{}", resp);
        finished.await;
    }
}
