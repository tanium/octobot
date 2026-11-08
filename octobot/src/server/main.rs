use std::net::SocketAddr;
use std::sync::Arc;

use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::{TokioIo, TokioTimer};
use log::{debug, error, info};
use octobot_events::pb::events_server::EventsServer;
use octobot_ops::event_bus::EventBus;
use octobot_ops::webhook_db::WebhookDatabase;
use tokio::net::TcpListener;
use tokio_stream::wrappers::TcpListenerStream;

use crate::runtime;
use crate::server::events::{EventsService, GithubKeys};
use crate::server::github_handler::GithubHandlerState;
use crate::server::octobot_service::OctobotService;
use crate::server::sessions::Sessions;
use octobot_lib::config::Config;
use octobot_lib::github;
use octobot_lib::jira;
use octobot_lib::jira::api::JiraSession;
use octobot_lib::metrics;

use octobot_lib::github::api::Session;

// How far a stream can fall behind before it gets a Reset.
const EVENT_LAG_CAPACITY: usize = 1_000;

pub fn start(config: Config) {
    let num_http_threads = config.main.num_http_threads.unwrap_or(20);
    let metrics = metrics::Metrics::new();

    runtime::run(num_http_threads, metrics.clone(), async move {
        run_server(config, metrics).await
    });
}

async fn run_server(config: Config, metrics: Arc<metrics::Metrics>) {
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

    let webhook_db = Arc::new(WebhookDatabase::new(&config.webhook_db_path()).expect("webhook db"));
    let latest_webhook_guid = webhook_db
        .get_latest_guid()
        .expect("failed to lookup latest guid");

    let jira_api: Option<Arc<dyn jira::api::Session>> = if let Some(ref jira_config) = config.jira {
        match JiraSession::new(jira_config, Some(metrics.clone())).await {
            Ok(s) => Some(Arc::new(s)),
            Err(e) => panic!("Error initiating jira session: {}", e),
        }
    } else {
        None
    };

    let http_addr: SocketAddr = match config.main.listen_addr {
        Some(ref addr_and_port) => addr_and_port.parse().unwrap(),
        None => "0.0.0.0:3000".parse().unwrap(),
    };

    let event_bus = Arc::new(EventBus::new(EVENT_LAG_CAPACITY));
    if let Some(ref events_config) = config.events {
        let service = EventsService::new(
            event_bus.clone(),
            &events_config.authority,
            Arc::new(GithubKeys(github_api.clone())),
            metrics.clone(),
        );
        let addr = &events_config.listen_addr;
        let listener = match TcpListener::bind(addr).await {
            Ok(l) => l,
            Err(e) => panic!("Error binding {}: {}", addr, e),
        };
        info!("Listening (events gRPC) on {}", addr);
        tokio::spawn(async move {
            let result = tonic::transport::Server::builder()
                .http2_keepalive_interval(Some(std::time::Duration::from_secs(30)))
                .add_service(EventsServer::new(service))
                .serve_with_incoming(TcpListenerStream::new(listener))
                .await;
            if let Err(e) = result {
                error!("Events server error: {}", e);
            }
        });
    }

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

    let jobs = tokio::spawn({
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

    serve(listener, octobot, metrics).await;

    let Err(e) = jobs.await;
    error!("jobs error: {}", e);

    if let Err(e) = webhook_redeliver.await {
        error!("webhook redeliver error: {}", e);
    }
}

async fn serve(listener: TcpListener, octobot: OctobotService, metrics: Arc<metrics::Metrics>) {
    loop {
        let stream = match listener.accept().await {
            Ok((stream, _)) => stream,
            Err(e) => {
                // Typically file descriptor exhaustion; back off instead of spinning.
                error!("Error accepting connection: {}", e);
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                continue;
            }
        };

        let scoped_count = metrics::scoped_inc(&metrics.current_connection_count);
        let octobot = octobot.clone();
        tokio::spawn(async move {
            let _scoped_count = scoped_count;

            let service = service_fn(move |req| octobot.clone().call(req));
            if let Err(e) = http1::Builder::new()
                .timer(TokioTimer::new())
                .serve_connection(TokioIo::new(stream), service)
                .await
            {
                debug!("Connection error: {}", e);
            }
        });
    }
}
