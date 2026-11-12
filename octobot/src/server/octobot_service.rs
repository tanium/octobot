use hyper::body::Incoming;
use hyper::{self, Method, Request, Response};
use log::{debug, info};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use octobot_lib::config::Config;
use octobot_lib::jira::api::Session as JiraSession;
use octobot_lib::metrics;
use octobot_ops::util;

use crate::http_util::Body;
use crate::server::admin;
use crate::server::admin::{Op, RepoAdmin, UserAdmin};
use crate::server::github_handler::{GithubHandler, GithubHandlerState};
use crate::server::http::{FilteredHandler, Handler, NotFoundHandler};
use crate::server::login::{LoginHandler, LoginSessionFilter, LogoutHandler, SessionCheckHandler};
use crate::server::metrics::MetricsScrapeHandler;
use crate::server::sessions::Sessions;
use crate::server::ui_handler::UiHandler;
use crate::server::version_handler::VersionHandler;

#[derive(Clone)]
pub struct OctobotService {
    config: Arc<Config>,
    ui_sessions: Arc<Sessions>,
    github_handler_state: Arc<GithubHandlerState>,
    jira_session: Option<Arc<dyn JiraSession>>,
    slack: Arc<octobot_ops::slack::Slack>,
    metrics: Arc<metrics::Metrics>,
}

impl OctobotService {
    pub fn new(
        config: Arc<Config>,
        ui_sessions: Arc<Sessions>,
        github_handler_state: Arc<GithubHandlerState>,
        jira_session: Option<Arc<dyn JiraSession>>,
        slack: Arc<octobot_ops::slack::Slack>,
        metrics: Arc<metrics::Metrics>,
    ) -> OctobotService {
        OctobotService {
            config,
            ui_sessions,
            github_handler_state,
            jira_session,
            slack,
            metrics,
        }
    }

    pub fn clean(&self) {
        self.github_handler_state.clean();

        let day = Duration::from_secs(24 * 3600);
        if let Err(e) = self.slack.clean(SystemTime::now() - 30 * day) {
            log::error!("Failed to clean slack db: {}", e);
        }
    }
}

impl OctobotService {
    pub async fn call(self, req: Request<Incoming>) -> Result<Response<Body>, hyper::Error> {
        let start = std::time::Instant::now();

        let method = req.method().clone();
        let path = req.uri().path().to_string();
        debug!("Received request: {} {}", method, path);
        let mut timer = metrics::RequestTimer::start(
            &self.metrics.http_duration,
            method.as_str(),
            &metrics::cleanup_path(&path),
        );

        let handler = self.route(&req);
        let res = handler.handle_ok(req).await;
        timer.respond(res.status().as_str());

        info!(
            "{} {} {} ({})",
            method,
            path,
            res.status(),
            util::format_duration(std::time::Instant::now() - start)
        );

        Ok(res)
    }

    fn route(&self, req: &Request<Incoming>) -> Box<dyn Handler> {
        // API routes
        if req.uri().path().starts_with("/api") {
            let filter = LoginSessionFilter::new(self.ui_sessions.clone());

            return FilteredHandler::new(
                filter,
                match (req.method(), req.uri().path()) {
                    (&Method::GET, "/api/users") => {
                        UserAdmin::new(self.config.clone(), self.slack.clone(), Op::List)
                    }
                    (&Method::PUT, "/api/user") => {
                        UserAdmin::new(self.config.clone(), self.slack.clone(), Op::Update)
                    }
                    (&Method::POST, "/api/users") => {
                        UserAdmin::new(self.config.clone(), self.slack.clone(), Op::Create)
                    }
                    (&Method::DELETE, "/api/user") => {
                        UserAdmin::new(self.config.clone(), self.slack.clone(), Op::Delete)
                    }
                    (&Method::POST, "/api/users/verify") => {
                        UserAdmin::new(self.config.clone(), self.slack.clone(), Op::Verify)
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
                        admin::MergeVersions::new(self.config.clone(), self.jira_session.clone())
                    }

                    (&Method::GET, "/api/version") => VersionHandler::new(),

                    _ => Box::new(NotFoundHandler),
                },
            );
        }

        // static routes
        match (req.method(), req.uri().path()) {
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

            // web ui
            (&Method::GET, _) => UiHandler::new(),

            _ => Box::new(NotFoundHandler),
        }
    }
}
