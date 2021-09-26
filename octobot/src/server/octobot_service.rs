use std::sync::Arc;
use futures::future::{self, Future};
use hyper::{self, Body, Method, Request};
use hyper::service::{NewService, Service};
use time;
use log::{debug, error, info};

use octobot_lib::config::Config;
use octobot_ops::util;

use crate::server::admin::{Op, RepoAdmin, UserAdmin};
use crate::server::admin;
use crate::server::github_handler::{GithubHandler, GithubHandlerState};
use crate::server::html_handler::HtmlHandler;
use crate::server::http::{FilteredHandler, FutureResponse, Handler, NotFoundHandler};
use crate::server::login::{LoginHandler, LoginSessionFilter, LogoutHandler, SessionCheckHandler};
use crate::server::sessions::Sessions;

#[derive(Clone)]
pub struct OctobotService {
    config: Arc<Config>,
    ui_sessions: Arc<Sessions>,
    github_handler_state: Arc<GithubHandlerState>,
}

impl OctobotService {
    pub fn new(
        config: Arc<Config>,
        ui_sessions: Arc<Sessions>,
        github_handler_state: Arc<GithubHandlerState>,
    ) -> OctobotService {
        OctobotService {
            config: config,
            ui_sessions: ui_sessions,
            github_handler_state: github_handler_state,
        }
    }
}

impl NewService for OctobotService {
    type ReqBody = Body;
    type ResBody = Body;
    type Error = hyper::Error;
    type Service = OctobotService;
    type Future = future::FutureResult<OctobotService, hyper::Error>;
    type InitError = hyper::Error;

    fn new_service(&self) -> Self::Future {
        future::ok(self.clone())
    }
}


impl Service for OctobotService {
    type ReqBody = Body;
    type ResBody = Body;
    type Error = hyper::Error;
    type Future = FutureResponse;

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        let start = time::now();

        let method = req.method().clone();
        let path = req.uri().path().to_string();
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
    fn route(&self, req: &Request<Body>) -> Box<dyn Handler> {
        // API routes
        if req.uri().path().starts_with("/api") {
            let filter = LoginSessionFilter::new(self.ui_sessions.clone());

            return FilteredHandler::new(
                filter,
                match (req.method(), req.uri().path()) {
                    (&Method::GET, "/api/users") => UserAdmin::new(self.config.clone(), Op::List),
                    (&Method::PUT, "/api/user") => UserAdmin::new(self.config.clone(), Op::Update),
                    (&Method::POST, "/api/users") => UserAdmin::new(self.config.clone(), Op::Create),
                    (&Method::DELETE, "/api/user") => UserAdmin::new(self.config.clone(), Op::Delete),

                    (&Method::GET, "/api/repos") => RepoAdmin::new(self.config.clone(), Op::List),
                    (&Method::PUT, "/api/repo") => RepoAdmin::new(self.config.clone(), Op::Update),
                    (&Method::POST, "/api/repos") => RepoAdmin::new(self.config.clone(), Op::Create),
                    (&Method::DELETE, "/api/repo") => RepoAdmin::new(self.config.clone(), Op::Delete),

                    (&Method::POST, "/api/merge-versions") => admin::MergeVersions::new(self.config.clone()),

                    _ => Box::new(NotFoundHandler),
                },
            );
        }

        // static routes
        match (req.method(), req.uri().path()) {
            // web ui resources. kinda a funny way of doing this maybe, but avoids worries about
            // path traversal and location of a doc root on deployment, and our resource count is small.
            (&Method::GET, "/") => HtmlHandler::new("index.html", include_str!("../../src/assets/index.html")),
            (&Method::GET, "/login.html") => {
                HtmlHandler::new("login.html", include_str!("../../src/assets/login.html"))
            }
            (&Method::GET, "/users.html") => {
                HtmlHandler::new("users.html", include_str!("../../src/assets/users.html"))
            }
            (&Method::GET, "/repos.html") => {
                HtmlHandler::new("repos.html", include_str!("../../src/assets/repos.html"))
            }
            (&Method::GET, "/versions.html") => {
                HtmlHandler::new("versions.html", include_str!("../../src/assets/versions.html"))
            }
            (&Method::GET, "/app.js") => HtmlHandler::new("app.js", include_str!("../../src/assets/app.js")),

            // auth
            (&Method::POST, "/auth/login") => LoginHandler::new(self.ui_sessions.clone(), self.config.clone()),
            (&Method::POST, "/auth/check") => SessionCheckHandler::new(self.ui_sessions.clone()),
            (&Method::POST, "/auth/logout") => LogoutHandler::new(self.ui_sessions.clone()),

            // hooks
            (&Method::POST, "/hooks/github") => GithubHandler::from_state(self.github_handler_state.clone()),

            _ => Box::new(NotFoundHandler),
        }
    }
}
