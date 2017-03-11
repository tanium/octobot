use std::sync::Arc;

use iron::prelude::*;
use router::Router;
use logger::Logger;

use config::Config;
use github;
use jira;
use server::github_verify;
use server::github_handler;

pub fn start(config: Config) -> Result<(), String> {
    let github_session = match github::api::GithubSession::new(&config.github.host,
                                                               &config.github.api_token) {
        Ok(s) => s,
        Err(e) => panic!("Error initiating github session: {}", e),
    };

    let config = Arc::new(config);

    let jira_session;
    if let Some(ref jira_config) = config.jira {
        jira_session = match jira::api::JiraSession::new(&jira_config.host, &jira_config.username, &jira_config.password) {
            Ok(s) => {
                let arc : Arc<jira::api::Session> = Arc::new(s);
                Some(arc)
            }
            Err(e) => panic!("Error initiating jira session: {}", e),
        };
    } else {
        jira_session = None;
    }

    let handler = github_handler::GithubHandler::new(config.clone(), github_session, jira_session);

    let mut router = Router::new();
    router.post("/hooks/github", handler, "github_webhook");

    let default_listen = String::from("0.0.0.0:3000");
    let addr_and_port = match config.main.listen_addr {
        Some(ref addr_and_port) => addr_and_port,
        None => &default_listen,
    };

    let mut chain = Chain::new(router);
    let (logger_before, logger_after) = Logger::new(None);

    // before first middleware
    chain.link_before(logger_before);

    chain.link_before( github_verify::GithubWebhookVerifier { secret: config.github.webhook_secret.clone() });

    // after last middleware
    chain.link_after(logger_after);

    match Iron::new(chain).http(addr_and_port.as_str()) {
        Ok(_) => {
            info!("Listening on port {}", addr_and_port);
            Ok(())
        }
        Err(e) => Err(format!("{}", e)),
    }
}
