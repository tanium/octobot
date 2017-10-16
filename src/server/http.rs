use std::sync::Arc;
use std::io::Read;

use iron::prelude::*;
use iron::middleware::{BeforeMiddleware, Handler};
use logger::Logger;

use config::Config;
use github;
use jira;
use server::github_handler::GithubHandler;
use server::github_verify::GithubWebhookVerifier;
use server::html_handler::HtmlHandler;
use server::login::{LoginHandler, LogoutHandler, LoginSessionFilter};
use server::admin;
use server::sessions::Sessions;

pub fn start(config: Config) -> Result<(), String> {
    let github_session = match github::api::GithubSession::new(&config.github.host,
                                                               &config.github.api_token) {
        Ok(s) => s,
        Err(e) => panic!("Error initiating github session: {}", e),
    };

    let config = Arc::new(config);

    let jira_session;
    if let Some(ref jira_config) = config.jira {
        jira_session = match jira::api::JiraSession::new(&jira_config) {
            Ok(s) => {
                let arc : Arc<jira::api::Session> = Arc::new(s);
                Some(arc)
            }
            Err(e) => panic!("Error initiating jira session: {}", e),
        };
    } else {
        jira_session = None;
    }

    let ui_sessions = Arc::new(Sessions::new());

    let router = router!(
        // web ui resources. kinda a funny way of doing this maybe, but avoids worries about
        // path traversal and location of a doc root on deployment, and our resource count is small.
        index: get "/"           => HtmlHandler::new("index.html", include_str!("../../src/assets/index.html")),
        login: get "/login.html" => HtmlHandler::new("login.html", include_str!("../../src/assets/login.html")),
        users: get "/users.html" => HtmlHandler::new("users.html", include_str!("../../src/assets/users.html")),
        repos: get "/repos.html" => HtmlHandler::new("repos.html", include_str!("../../src/assets/repos.html")),
        versions: get "/versions.html" => HtmlHandler::new("versions.html", include_str!("../../src/assets/versions.html")),
        app_js: get "/app.js"    => HtmlHandler::new("app.js", include_str!("../../src/assets/app.js")),
        // auth
        auth_login:  post "/auth/login"  => LoginHandler::new(ui_sessions.clone(), config.clone()),
        auth_logout: post "/auth/logout" => LogoutHandler::new(ui_sessions.clone()),
        // api
        api: any "/api/*" => filtered_handler(
            LoginSessionFilter::new(ui_sessions.clone()),
            router!(
                api_get_users:    get  "/api/users" => admin::GetUsers::new(config.clone()),
                api_update_users: post "/api/users" => admin::UpdateUsers::new(config.clone()),
                api_get_repos:    get  "/api/repos" => admin::GetRepos::new(config.clone()),
                api_update_repos: post "/api/repos" => admin::UpdateRepos::new(config.clone()),
                api_merge_versions: post "/api/merge-versions" => admin::MergeVersions::new(config.clone(), jira_session.clone()),
            )
        ),
        // hooks
        hooks_github: post "/hooks/github" => filtered_handler(
            GithubWebhookVerifier { secret: config.github.webhook_secret.clone() },
            GithubHandler::new(config.clone(), github_session, jira_session)
        ),
    );

    let default_listen = String::from("0.0.0.0:3000");
    let addr_and_port = match config.main.listen_addr {
        Some(ref addr_and_port) => addr_and_port,
        None => &default_listen,
    };

    let mut chain = Chain::new(router);
    let (logger_before, logger_after) = Logger::new(None);

    // before first middleware
    chain.link_before(logger_before);

    // after last middleware
    chain.link_after(logger_after);

    // Read off rest of the request body in case it wasn't used
    // to fix issue where it appears to affect the next request
    // (may be an http pipelining bug in iron?)
    chain.link_after(|req: &mut Request, resp: Response| {
        let mut buffer = [0; 8192];
        loop {
            match req.body.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(read) => {
                    info!("Unused data! {} bytes", read);
                }
            }
        }
        Ok(resp)
    });

    match Iron::new(chain).http(addr_and_port.as_str()) {
        Ok(_) => {
            info!("Listening on port {}", addr_and_port);
            Ok(())
        }
        Err(e) => Err(format!("{}", e)),
    }
}

fn filtered_handler<F: BeforeMiddleware, H: Handler>(filter: F, handler: H) -> Chain {
    let mut chain = Chain::new(handler);
    chain.link_before(filter);
    chain
}
