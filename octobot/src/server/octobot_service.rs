use hyper::{self, Body, Method, Request, Response};
use log::{debug, info};
use maplit::hashmap;
use std::sync::Arc;

use octobot_lib::config::Config;
use octobot_lib::metrics;
use octobot_ops::util;

use crate::server::admin;
use crate::server::admin::{Op, RepoAdmin, UserAdmin};
use crate::server::github_handler::{GithubHandler, GithubHandlerState};
use crate::server::html_handler::HtmlHandler;
use crate::server::http::{FilteredHandler, Handler, NotFoundHandler};
use crate::server::login::{LoginHandler, LoginSessionFilter, LogoutHandler, SessionCheckHandler};
use crate::server::metrics::MetricsScrapeHandler;
use crate::server::sessions::Sessions;

#[derive(Clone)]
pub struct OctobotService {
    config: Arc<Config>,
    ui_sessions: Arc<Sessions>,
    github_handler_state: Arc<GithubHandlerState>,
    metrics: Arc<metrics::Metrics>,
}

impl OctobotService {
    pub fn new(
        config: Arc<Config>,
        ui_sessions: Arc<Sessions>,
        github_handler_state: Arc<GithubHandlerState>,
        metrics: Arc<metrics::Metrics>,
    ) -> OctobotService {
        OctobotService {
            config,
            ui_sessions,
            github_handler_state,
            metrics,
        }
    }
}

impl OctobotService {
    pub async fn call(self, req: Request<Body>) -> Result<Response<Body>, hyper::Error> {
        let start = std::time::Instant::now();

        let method = req.method().clone();
        let path = req.uri().path().to_string();
        debug!("Received request: {} {}", method, path);

        let metrics_path = &metrics::cleanup_path(req.uri().path());
        let _timer = self
            .metrics
            .http_duration
            .with(&hashmap! {
                    "method" => req.method().as_str(),
                    "path" => metrics_path,
            })
            .start_timer();

        let handler = self.route(&req);
        let res = handler.handle_ok(req).await;

        info!(
            "{} {} {} ({})",
            method,
            path,
            res.status(),
            util::format_duration(std::time::Instant::now() - start)
        );

        Ok(res)
    }

    fn route(&self, req: &Request<Body>) -> Box<dyn Handler> {
        // API routes
        if req.uri().path().starts_with("/api") {
            let filter = LoginSessionFilter::new(self.ui_sessions.clone());

            return FilteredHandler::new(
                filter,
                match (req.method(), req.uri().path()) {
                    (&Method::GET, "/api/users") => UserAdmin::new(self.config.clone(), Op::List),
                    (&Method::PUT, "/api/user") => UserAdmin::new(self.config.clone(), Op::Update),
                    (&Method::POST, "/api/users") => {
                        UserAdmin::new(self.config.clone(), Op::Create)
                    }
                    (&Method::DELETE, "/api/user") => {
                        UserAdmin::new(self.config.clone(), Op::Delete)
                    }

                    (&Method::GET, "/api/repos") => RepoAdmin::new(self.config.clone(), Op::List),
                    (&Method::PUT, "/api/repo") => RepoAdmin::new(self.config.clone(), Op::Update),
                    (&Method::POST, "/api/repos") => {
                        RepoAdmin::new(self.config.clone(), Op::Create)
                    }
                    (&Method::DELETE, "/api/repo") => {
                        RepoAdmin::new(self.config.clone(), Op::Delete)
                    }

                    (&Method::POST, "/api/merge-versions") => {
                        admin::MergeVersions::new(self.config.clone())
                    }

                    _ => Box::new(NotFoundHandler),
                },
            );
        }

        // static routes
        match (req.method(), req.uri().path()) {
            // web ui resources. kinda a funny way of doing this maybe, but avoids worries about
            // path traversal and location of a doc root on deployment, and our resource count is small.
            (&Method::GET, "/") => {
                HtmlHandler::new("index.html", include_str!("../../src/assets/index.html"))
            }
            (&Method::GET, "/login.html") => {
                HtmlHandler::new("login.html", include_str!("../../src/assets/login.html"))
            }
            (&Method::GET, "/users.html") => {
                HtmlHandler::new("users.html", include_str!("../../src/assets/users.html"))
            }
            (&Method::GET, "/repos.html") => {
                HtmlHandler::new("repos.html", include_str!("../../src/assets/repos.html"))
            }
            (&Method::GET, "/versions.html") => HtmlHandler::new(
                "versions.html",
                include_str!("../../src/assets/versions.html"),
            ),
            (&Method::GET, "/app.js") => {
                HtmlHandler::new("app.js", include_str!("../../src/assets/app.js"))
            }

            // auth
            (&Method::POST, "/auth/login") => {
                LoginHandler::new(self.ui_sessions.clone(), self.config.clone())
            }
            (&Method::POST, "/auth/check") => SessionCheckHandler::new(self.ui_sessions.clone()),
            (&Method::POST, "/auth/logout") => LogoutHandler::new(self.ui_sessions.clone()),

            // hooks
            (&Method::POST, "/hooks/github") => {
                GithubHandler::from_state(self.github_handler_state.clone())
            }

            // metrics
            (&Method::GET, "/metrics") => {
                MetricsScrapeHandler::new(self.config.clone(), self.metrics.clone())
            }

            _ => Box::new(NotFoundHandler),
        }
    }
}
