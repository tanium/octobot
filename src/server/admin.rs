use std::borrow::Borrow;
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::sync::Arc;

use hyper::StatusCode;
use hyper::header::ContentType;
use hyper::server::{Request, Response};
use serde_json;
use tokio_core::reactor::Remote;

use config::{Config, JiraConfig};
use jira;
use repos::RepoHostMap;
use server::http::{FutureResponse, Handler, parse_json};
use users::UserInfo;
use util;
use version;

pub enum Op {
    List,
    Create,
    Update,
    Delete,
}

pub struct UserAdmin {
    config: Arc<Config>,
    op: Op,
}

pub struct GetRepos {
    config: Arc<Config>,
}

pub struct UpdateRepos {
    config: Arc<Config>,
}

impl UserAdmin {
    pub fn new(config: Arc<Config>, op: Op) -> Box<UserAdmin> {
        Box::new(UserAdmin {
            config: config,
            op: op,
        })
    }
}

impl GetRepos {
    pub fn new(config: Arc<Config>) -> Box<GetRepos> {
        Box::new(GetRepos { config: config })
    }
}

impl UpdateRepos {
    pub fn new(config: Arc<Config>) -> Box<UpdateRepos> {
        Box::new(UpdateRepos { config: config })
    }
}

impl Handler for UserAdmin {
    fn handle(&self, req: Request) -> FutureResponse {
        match &self.op {
            &Op::List => self.get_all(req),
            &Op::Create => self.create(req),
            &Op::Update => self.update(req),
            &Op::Delete => self.delete(req),
        }
    }
}

impl UserAdmin {
    fn get_all(&self, _: Request) -> FutureResponse {
        #[derive(Serialize)]
        struct UsersResp {
            users: Vec<UserInfo>,
        }

        let users = match self.config.users().get_all() {
            Ok(u) => u,
            Err(e) => {
                return self.respond_error(&format!("{}", e));
            }
        };
        let resp = UsersResp { users: users };

        let users = match serde_json::to_string(&resp) {
            Ok(u) => u,
            Err(e) => {
                error!("Error serializing users: {}", e);
                String::new()
            }
        };
        self.respond(Response::new().with_header(ContentType::json()).with_body(users))
    }

    fn create(&self, req: Request) -> FutureResponse {
        #[derive(Deserialize)]
        struct NewUserReq {
            github: String,
            slack: String,
        }

        let config = self.config.clone();
        parse_json(req, move |user: NewUserReq| {
            if let Err(e) = config.users_write().insert(String::new(), user.github, user.slack) {
                error!("{}", e);
                return Response::new().with_status(StatusCode::InternalServerError);
            }
            Response::new().with_status(StatusCode::Ok)
        })
    }

    fn update(&self, req: Request) -> FutureResponse {
        let config = self.config.clone();

        parse_json(req, move |user: UserInfo| {
            if let Err(e) = config.users_write().update(&user) {
                error!("{}", e);
                return Response::new().with_status(StatusCode::InternalServerError);
            }
            Response::new().with_status(StatusCode::Ok)
        })
    }

    fn delete(&self, req: Request) -> FutureResponse {
        let config = self.config.clone();

        let query = util::parse_query(req.uri().query());
        println!("QUERIES! {:?}", query);

        let user_id = match query.get("id").map(|id| id.parse::<i32>()) {
            None | Some(Err(_)) => return self.respond_with(StatusCode::BadRequest, "No `id` param specified"),
            Some(Ok(id)) => id,
        };

        if let Err(e) = config.users_write().delete(user_id) {
            return self.respond_error(&format!("{}", e));
        }
        self.respond_with(StatusCode::Ok, "")
    }
}

impl Handler for GetRepos {
    fn handle(&self, _: Request) -> FutureResponse {
        let repos = match serde_json::to_string(&*self.config.repos()) {
            Ok(u) => u,
            Err(e) => {
                error!("Error serializing repos: {}", e);
                String::new()
            }
        };
        self.respond(Response::new().with_header(ContentType::json()).with_body(repos))
    }
}

impl Handler for UpdateRepos {
    fn handle(&self, req: Request) -> FutureResponse {
        let config = self.config.clone();

        parse_json(req, move |repos: RepoHostMap| {
            let json = match serde_json::to_string_pretty(&repos) {
                Ok(j) => j,
                Err(e) => {
                    error!("Error serializing repos: {}", e);
                    return Response::new().with_status(StatusCode::InternalServerError).with_body(format!(
                        "Error serializing JSON: {}",
                        e
                    ));
                }
            };

            let config_file = config.main.repos_config_file.clone();
            let config_file_tmp = config_file.clone() + ".tmp";

            let mut file = match fs::File::create(&config_file_tmp) {
                Ok(f) => f,
                Err(e) => {
                    error!("Error opening file: {}", e);
                    return Response::new().with_status(StatusCode::InternalServerError).with_body(format!(
                        "Error opening file: {}",
                        e
                    ));
                }
            };

            if let Err(e) = file.write_all(json.as_bytes()) {
                error!("Error writing file: {}", e);
                return Response::new().with_status(StatusCode::InternalServerError).with_body(format!(
                    "Error writing file: {}",
                    e
                ));
            }

            if let Err(e) = fs::rename(&config_file_tmp, &config_file) {
                error!("Error renaming file: {}", e);
                return Response::new().with_status(StatusCode::InternalServerError).with_body(format!(
                    "Error renaming file: {}",
                    e
                ));
            }

            if let Err(e) = config.reload_users_repos() {
                error!("Error reloading config: {}", e);
            }

            Response::new()
        })
    }
}

pub struct MergeVersions {
    config: Arc<Config>,
    core_remote: Remote,
}

impl MergeVersions {
    pub fn new(config: Arc<Config>, core_remote: Remote) -> Box<MergeVersions> {
        Box::new(MergeVersions {
            config: config,
            core_remote: core_remote,
        })
    }
}

#[derive(Deserialize, Clone)]
struct MergeVersionsReq {
    admin_user: Option<String>,
    admin_pass: Option<String>,
    project: String,
    version: String,
    dry_run: bool,
}

#[derive(Serialize, Clone)]
struct MergeVersionsResp {
    jira_base: String,
    versions: HashMap<String, Vec<version::Version>>,
}

impl Handler for MergeVersions {
    fn handle(&self, req: Request) -> FutureResponse {
        let config = self.config.clone();
        let core_remote = self.core_remote.clone();
        parse_json(req, move |merge_req: MergeVersionsReq| {
            // make a copy of the jira config so we can modify the auth
            let mut jira_config: JiraConfig = match config.jira {
                Some(ref j) => j.clone(),
                None => return Response::new().with_status(StatusCode::BadRequest).with_body("No JIRA config"),
            };

            if !merge_req.dry_run {
                jira_config.username = merge_req.admin_user.unwrap_or(String::new());
                jira_config.password = merge_req.admin_pass.unwrap_or(String::new());

                if jira_config.username.is_empty() || jira_config.password.is_empty() {
                    return Response::new().with_status(StatusCode::BadRequest).with_body(
                        "JIRA auth required for non dry-run",
                    );
                }
            }

            let jira_sess = match jira::api::JiraSession::new(core_remote, &jira_config) {
                Ok(j) => j,
                Err(e) => {
                    return Response::new().with_status(StatusCode::BadRequest).with_body(format!(
                        "Error creating JIRA session: {}",
                        e
                    ))
                }
            };

            let dry_run_mode = if merge_req.dry_run {
                jira::workflow::DryRunMode::DryRun
            } else {
                jira::workflow::DryRunMode::ForReal
            };

            let all_relevant_versions = match jira::workflow::merge_pending_versions(
                &merge_req.version,
                &merge_req.project,
                jira_sess.borrow(),
                dry_run_mode,
            ) {
                Ok(v) => v,
                Err(e) => {
                    error!("Error merging pending versions: {}", e);
                    return Response::new().with_status(StatusCode::InternalServerError).with_body(format!(
                        "Error merging pending versions: {}",
                        e
                    ));
                }
            };

            if !merge_req.dry_run {
                if let Err(e) = jira::workflow::sort_versions(&merge_req.project, jira_sess.borrow()) {
                    error!("Error sorting versions: {}", e);
                }
            }

            let resp = MergeVersionsResp {
                jira_base: jira_config.base_url(),
                versions: all_relevant_versions,
            };

            let resp_json = match serde_json::to_string(&resp) {
                Ok(r) => r,
                Err(e) => {
                    error!("Error serializing versions: {}", e);
                    return Response::new().with_status(StatusCode::InternalServerError).with_body(format!(
                        "Error serializing pending versions: {}",
                        e
                    ));
                }
            };

            Response::new().with_header(ContentType::json()).with_body(resp_json)
        })
    }
}
