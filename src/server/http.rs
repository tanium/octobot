use std::sync::Arc;

use futures::future::{self, Future};
use futures::Stream;
use hyper::{self, StatusCode};
use hyper::server::{Http, Request, Response, Service};
use serde::de::DeserializeOwned;
use serde_json;
use time;

use config::Config;
use github;
use github::api::GithubSession;
use jira;
use jira::api::JiraSession;
use server::github_handler::GithubHandler;
use server::html_handler::HtmlHandler;
use server::login::{LoginHandler, LogoutHandler, LoginSessionFilter};
use server::admin;
use server::sessions::Sessions;
use util;

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

struct OctobotService {
    config: Arc<Config>,
    github: Arc<github::api::Session>,
    jira: Option<Arc<jira::api::Session>>,
    ui_sessions: Arc<Sessions>,
}

impl OctobotService {
    pub fn new(config: Arc<Config>,
               github: Arc<github::api::Session>,
               jira: Option<Arc<jira::api::Session>>,
               ui_sessions: Arc<Sessions>) -> OctobotService {
        OctobotService {
            config: config,
            github: github,
            jira: jira,
            ui_sessions: ui_sessions,
        }
    }
}

pub type FutureResponse = Box<Future<Item=Response, Error=hyper::Error>>;

pub trait OctobotHandler {
    fn handle(&self, req: Request) -> FutureResponse;

    fn respond(&self, resp: Response) -> FutureResponse {
        Box::new(future::ok(resp))
    }

    fn respond_with(&self, status: hyper::StatusCode, msg: &str) -> FutureResponse {
        self.respond(Response::new().with_status(status).with_body(msg.to_string()))
    }
}

pub fn parse_json<T: DeserializeOwned, F>(req: Request, func: F) -> FutureResponse
    where F: FnOnce(T) -> Response + 'static
{
    Box::new(req.body().concat2().map(move |data| {
        let obj: T = match serde_json::from_slice(&data) {
            Ok(l) => l,
            Err(e) => return Response::new().with_status(StatusCode::BadRequest)
                                            .with_body(format!("Failed to parse JSON: {}", e)),
        };

        func(obj)
    }))
}

pub trait OctobotFilter {
    fn filter(&self, req: &Request) -> OctobotFilterResult;
}

pub enum OctobotFilterResult {
    Halt(Response),
    Continue,
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

        Box::new(self.route(&req).handle(req).map(move |res| {
            info!("{} {} {} ({})", method, path, res.status(), util::format_duration(time::now() - start));
            res
        }).or_else(move |e| {
            error!("Error processing request: {}", e);
            future::err(e)
        }))
    }
}

impl OctobotService {
    fn route(&self, req: &Request) -> Box<OctobotHandler> {
        use hyper::Method::{Get, Post};

        // API routes
        if req.path().starts_with("/api") {
            let filter = LoginSessionFilter::new(self.ui_sessions.clone());

            return FilteredHandler::new(filter, match (req.method(), req.path()) {
                (&Get,  "/api/users") => admin::GetUsers::new(self.config.clone()),
                (&Post, "/api/users") => admin::UpdateUsers::new(self.config.clone()),
                (&Get,  "/api/repos") => admin::GetRepos::new(self.config.clone()),
                (&Post, "/api/repos") => admin::UpdateRepos::new(self.config.clone()),
                (&Post, "/api/merge-versions") => admin::MergeVersions::new(self.config.clone()),

                _ => Box::new(NotFoundHandler),
            });
        }

        // static routes
        match (req.method(), req.path()) {
            // web ui resources. kinda a funny way of doing this maybe, but avoids worries about
            // path traversal and location of a doc root on deployment, and our resource count is small.
            (&Get, "/")              => HtmlHandler::new("index.html", include_str!("../../src/assets/index.html")),
            (&Get, "/login.html")    => HtmlHandler::new("login.html", include_str!("../../src/assets/login.html")),
            (&Get, "/users.html")    => HtmlHandler::new("users.html", include_str!("../../src/assets/users.html")),
            (&Get, "/repos.html")    => HtmlHandler::new("repos.html", include_str!("../../src/assets/repos.html")),
            (&Get, "/versions.html") => HtmlHandler::new("versions.html", include_str!("../../src/assets/versions.html")),
            (&Get, "/app.js")        => HtmlHandler::new("app.js", include_str!("../../src/assets/app.js")),

            // auth
            (&Post, "/auth/login")   => LoginHandler::new(self.ui_sessions.clone(), self.config.clone()),
            (&Post, "/auth/logout")  => LogoutHandler::new(self.ui_sessions.clone()),

            // hooks
            (&Post, "/hooks/github") => GithubHandler::new(self.config.clone(), self.github.clone(), self.jira.clone()),

            _ => Box::new(NotFoundHandler),
        }
    }
}

struct NotFoundHandler;
impl OctobotHandler for NotFoundHandler {
    fn handle(&self, _: Request) -> FutureResponse {
        Box::new(future::ok(Response::new().with_status(StatusCode::NotFound)))
    }
}

struct FilteredHandler {
    filter: Box<OctobotFilter>,
    handler: Box<OctobotHandler>,
}

impl FilteredHandler {
    pub fn new(filter: Box<OctobotFilter>, handler: Box<OctobotHandler>) -> Box<FilteredHandler> {
        Box::new(FilteredHandler {
            filter: filter,
            handler: handler,
        })
    }
}

impl OctobotHandler for FilteredHandler {
    fn handle(&self, req: Request) -> FutureResponse {
        match self.filter.filter(&req) {
            OctobotFilterResult::Halt(resp) => Box::new(future::ok(resp)),
            OctobotFilterResult::Continue => self.handler.handle(req),
        }
    }
}

