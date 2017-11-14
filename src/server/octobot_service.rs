use std::sync::Arc;

use futures::future::{self, Future};
use hyper;
use hyper::server::{Request, Response, Service};
use time;
use tokio_core::reactor::Remote;

use config::Config;
use server::admin;
use server::admin::{Op, RepoAdmin, UserAdmin};
use server::github_handler::{GithubHandler, GithubHandlerState};
use server::html_handler::HtmlHandler;
use server::http::{FilteredHandler, FutureResponse, Handler, NotFoundHandler};
use server::login::{LoginHandler, LoginSessionFilter, LogoutHandler, SessionCheckHandler};
use server::sessions::Sessions;
use util;

pub struct OctobotService {
    config: Arc<Config>,
    ui_sessions: Arc<Sessions>,
    github_handler_state: Arc<GithubHandlerState>,
    core_remote: Remote,
}

impl OctobotService {
    pub fn new(
        config: Arc<Config>,
        ui_sessions: Arc<Sessions>,
        github_handler_state: Arc<GithubHandlerState>,
        core_remote: Remote,
    ) -> OctobotService {
        OctobotService {
            config: config,
            ui_sessions: ui_sessions,
            github_handler_state: github_handler_state,
            core_remote: core_remote,
        }
    }
}

impl Service for OctobotService {
    type Request = Request;
    type Response = Response;
    type Error = hyper::Error;
    type Future = FutureResponse;

    fn call(&self, req: Request) -> Self::Future {
        let start = time::now();

        let method = req.method().clone();
        let path = req.path().to_string();
        debug!("Received request: {} {}", method, path);

        Box::new(
            self.route(&req)
                .handle(req)
                .map(move |res| {
                    info!("{} {} {} ({})", method, path, res.status(), util::format_duration(time::now() - start));
                    res
                })
                .or_else(move |e| {
                    error!("Error processing request: {}", e);
                    future::err(e)
                }),
        )
    }
}

impl OctobotService {
    fn route(&self, req: &Request) -> Box<Handler> {
        use hyper::Method::{Delete, Get, Post, Put};

        // API routes
        if req.path().starts_with("/api") {
            let filter = LoginSessionFilter::new(self.ui_sessions.clone());

            return FilteredHandler::new(
                filter,
                match (req.method(), req.path()) {
                    (&Get, "/api/users") => UserAdmin::new(self.config.clone(), Op::List),
                    (&Put, "/api/user") => UserAdmin::new(self.config.clone(), Op::Update),
                    (&Post, "/api/users") => UserAdmin::new(self.config.clone(), Op::Create),
                    (&Delete, "/api/user") => UserAdmin::new(self.config.clone(), Op::Delete),

                    (&Get, "/api/repos") => RepoAdmin::new(self.config.clone(), Op::List),
                    (&Put, "/api/repo") => RepoAdmin::new(self.config.clone(), Op::Update),
                    (&Post, "/api/repos") => RepoAdmin::new(self.config.clone(), Op::Create),
                    (&Delete, "/api/repo") => RepoAdmin::new(self.config.clone(), Op::Delete),

                    (&Post, "/api/merge-versions") => {
                        admin::MergeVersions::new(self.config.clone(), self.core_remote.clone())
                    }

                    _ => Box::new(NotFoundHandler),
                },
            );
        }

        // static routes
        match (req.method(), req.path()) {
            // web ui resources. kinda a funny way of doing this maybe, but avoids worries about
            // path traversal and location of a doc root on deployment, and our resource count is small.
            (&Get, "/") => HtmlHandler::new("index.html", include_str!("../../src/assets/index.html")),
            (&Get, "/login.html") => HtmlHandler::new("login.html", include_str!("../../src/assets/login.html")),
            (&Get, "/users.html") => HtmlHandler::new("users.html", include_str!("../../src/assets/users.html")),
            (&Get, "/repos.html") => HtmlHandler::new("repos.html", include_str!("../../src/assets/repos.html")),
            (&Get, "/versions.html") => {
                HtmlHandler::new("versions.html", include_str!("../../src/assets/versions.html"))
            }
            (&Get, "/app.js") => HtmlHandler::new("app.js", include_str!("../../src/assets/app.js")),

            // auth
            (&Post, "/auth/login") => LoginHandler::new(self.ui_sessions.clone(), self.config.clone()),
            (&Post, "/auth/check") => SessionCheckHandler::new(self.ui_sessions.clone()),
            (&Post, "/auth/logout") => LogoutHandler::new(self.ui_sessions.clone()),

            // hooks
            (&Post, "/hooks/github") => GithubHandler::from_state(self.github_handler_state.clone()),

            _ => Box::new(NotFoundHandler),
        }
    }
}
