use super::*;

use std::sync::Arc;
use super::iron::prelude::*;
use super::router::Router;
use super::super::logger::Logger;

use super::super::users;
use super::super::repos;
use super::super::github;
use server::github_verify;
use server::github_handler;

pub fn start(config: Config) -> Result<(), String> {
    let user_config = match users::load_config(&config.users_config_file) {
        Ok(c) => c,
        Err(e) => panic!("Error reading user config file: {}", e),
    };
    let repo_config = match repos::load_config(&config.repos_config_file) {
        Ok(c) => c,
        Err(e) => panic!("Error reading repo config file: {}", e),
    };

    let github_session = match github::api::Session::new(&config.github_host, &config.github_token) {
        Ok(s) => s,
        Err(e) => panic!("Error initiating github session: {}", e),
    };

    let handler = github_handler::GithubHandler {
        users: Arc::new(user_config.clone()),
        repos: Arc::new(repo_config.clone()),
        config: Arc::new(config.clone()),
        github_session: Arc::new(github_session),
    };

    let mut router = Router::new();
    router.post("/", handler, "webhook");

    let default_listen = String::from("0.0.0.0:3000");
    let addr_and_port = match config.listen_addr {
        Some(ref addr_and_port) => addr_and_port,
        None => &default_listen,
    };

    let mut chain = Chain::new(router);
    let (logger_before, logger_after) = Logger::new(None);

    // before first middleware
    chain.link_before(logger_before);

    chain.link_before(github_verify::GithubWebhookVerifier { secret: config.github_secret.clone() });

    // after last middleware
    chain.link_after(logger_after);

    match Iron::new(chain).http(addr_and_port.as_str()) {
        Ok(_) => {
            println!("Listening on port {}", addr_and_port);
            Ok(())
        }
        Err(e) => Err(format!("{}", e)),
    }
}
