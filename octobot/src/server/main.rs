use std::net::SocketAddr;
use std::sync::Arc;

use hyper::server::Server;
use hyper::service::{make_service_fn, service_fn};
use log::{error, info};

use crate::runtime;
use crate::server::github_handler::GithubHandlerState;
use crate::server::octobot_service::OctobotService;
use crate::server::sessions::Sessions;
use octobot_lib::config::Config;
use octobot_lib::github;
use octobot_lib::jira;
use octobot_lib::jira::api::JiraSession;
use octobot_lib::metrics;

pub fn start(config: Config) {
    let num_http_threads = config.main.num_http_threads.unwrap_or(20);
    let metrics = metrics::Metrics::new();

    runtime::run(num_http_threads, metrics.clone(), async move {
        run_server(config, metrics).await
    });
}

async fn run_server(config: Config, metrics: Arc<metrics::Metrics>) {
    let config = Arc::new(config);

    let github: Arc<dyn github::api::GithubSessionFactory>;

    if config.github.app_id.is_some() {
        github = match github::api::GithubApp::new(
            &config.github.host,
            config.github.app_id.expect("expected an app_id"),
            &config.github.app_key().expect("expected an app_key"),
            Some(metrics.clone()),
        )
        .await
        {
            Ok(s) => Arc::new(s),
            Err(e) => panic!("Error initiating github session: {}", e),
        };
    } else {
        github = match github::api::GithubOauthApp::new(
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
        };
    }

    let jira: Option<Arc<dyn jira::api::Session>>;
    if let Some(ref jira_config) = config.jira {
        jira = match JiraSession::new(jira_config, Some(metrics.clone())).await {
            Ok(s) => Some(Arc::new(s)),
            Err(e) => panic!("Error initiating jira session: {}", e),
        };
    } else {
        jira = None;
    }

    let http_addr: SocketAddr = match config.main.listen_addr {
        Some(ref addr_and_port) => addr_and_port.parse().unwrap(),
        None => "0.0.0.0:3000".parse().unwrap(),
    };

    let ui_sessions = Arc::new(Sessions::new());
    let github_handler_state = Arc::new(GithubHandlerState::new(
        config.clone(),
        github.clone(),
        jira.clone(),
        metrics.clone(),
    ));
    let octobot = OctobotService::new(
        config.clone(),
        ui_sessions.clone(),
        github_handler_state.clone(),
        metrics.clone(),
    );

    let main_service = make_service_fn(move |_| {
        let metrics = metrics.clone();
        let _scoped_count = metrics::scoped_inc(&metrics.current_connection_count);

        let octobot = octobot.clone();

        async move {
            // move the scoped count inside the future
            let _scoped_count = _scoped_count;

            let octobot = octobot.clone();
            Ok::<_, hyper::Error>(service_fn(move |req| {
                let octobot = octobot.clone();
                octobot.call(req)
            }))
        }
    });

    let server = Server::bind(&http_addr).serve(main_service);
    info!("Listening (HTTP) on {}", http_addr);

    if let Err(e) = server.await {
        error!("server error: {}", e);
    }
}
