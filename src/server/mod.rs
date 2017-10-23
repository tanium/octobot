mod admin;
pub mod github_handler;
mod github_verify;
mod html_handler;
mod http;
mod octobot_service;
pub mod login;
mod sessions;


use hyper::server::{Http};

use std::sync::Arc;
use config::Config;
use github;
use github::api::GithubSession;
use jira;
use jira::api::JiraSession;
use self::octobot_service::OctobotService;
use self::sessions::Sessions;


pub fn start(config: Config) -> Result<(), String> {
    let config = Arc::new(config);

    let github: Arc<github::api::Session> =
            match GithubSession::new(&config.github.host, &config.github.api_token) {
        Ok(s) => Arc::new(s),
        Err(e) => panic!("Error initiating github session: {}", e),
    };

    let jira: Option<Arc<jira::api::Session>>;
    if let Some(ref jira_config) = config.jira {
        jira = match JiraSession::new(&jira_config) {
            Ok(s) => Some(Arc::new(s)),
            Err(e) => panic!("Error initiating jira session: {}", e),
        };
    } else {
        jira = None;
    }

    let addr = match config.main.listen_addr {
        Some(ref addr_and_port) => addr_and_port.parse().unwrap(),
        None => "0.0.0.0:3000".parse().unwrap(),
    };

    let ui_sessions = Arc::new(Sessions::new());

    let server = Http::new().bind(&addr, move || {
        Ok(OctobotService::new(config.clone(), github.clone(), jira.clone(), ui_sessions.clone()))
    }).unwrap();

    info!("Listening on port {}", addr);
    server.run().unwrap();

    Ok(())
}
