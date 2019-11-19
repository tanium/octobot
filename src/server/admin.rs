use std::borrow::Borrow;
use std::collections::HashMap;
use std::sync::Arc;

use hyper::{Body, Request};
use hyper::StatusCode;
use serde_json;
use serde_derive::{Deserialize, Serialize};
use log::error;

use crate::config::{Config, JiraConfig};
use crate::jira;
use crate::repos::RepoInfo;
use crate::server::http::{FutureResponse, Handler, parse_json};
use crate::users::UserInfo;
use crate::util;
use crate::version;

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

pub struct RepoAdmin {
    config: Arc<Config>,
    op: Op,
}


impl UserAdmin {
    pub fn new(config: Arc<Config>, op: Op) -> Box<UserAdmin> {
        Box::new(UserAdmin {
            config: config,
            op: op,
        })
    }
}

impl RepoAdmin {
    pub fn new(config: Arc<Config>, op: Op) -> Box<RepoAdmin> {
        Box::new(RepoAdmin {
            config: config,
            op: op,
        })
    }
}


impl Handler for UserAdmin {
    fn handle(&self, req: Request<Body>) -> FutureResponse {
        match &self.op {
            &Op::List => self.get_all(req),
            &Op::Create => self.create(req),
            &Op::Update => self.update(req),
            &Op::Delete => self.delete(req),
        }
    }
}

impl UserAdmin {
    fn get_all(&self, _: Request<Body>) -> FutureResponse {
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
        self.respond(util::new_json_resp(users))
    }

    fn create(&self, req: Request<Body>) -> FutureResponse {
        let config = self.config.clone();
        parse_json(req, move |user: UserInfo| {
            if let Err(e) = config.users_write().insert_info(&user) {
                error!("{}", e);
                return util::new_empty_error_resp();
            }
            util::new_empty_resp(StatusCode::OK)
        })
    }

    fn update(&self, req: Request<Body>) -> FutureResponse {
        let config = self.config.clone();

        parse_json(req, move |user: UserInfo| {
            if let Err(e) = config.users_write().update(&user) {
                error!("{}", e);
                return util::new_empty_error_resp();
            }
            util::new_empty_resp(StatusCode::OK)
        })
    }

    fn delete(&self, req: Request<Body>) -> FutureResponse {
        let config = self.config.clone();

        let query = util::parse_query(req.uri().query());

        let user_id = match query.get("id").map(|id| id.parse::<i32>()) {
            None | Some(Err(_)) => return self.respond(util::new_bad_req_resp("No `id` param specified")),
            Some(Ok(id)) => id,
        };

        if let Err(e) = config.users_write().delete(user_id) {
            return self.respond_error(&format!("{}", e));
        }
        self.respond_with(StatusCode::OK, "")
    }
}

impl Handler for RepoAdmin {
    fn handle(&self, req: Request<Body>) -> FutureResponse {
        match &self.op {
            &Op::List => self.get_all(req),
            &Op::Create => self.create(req),
            &Op::Update => self.update(req),
            &Op::Delete => self.delete(req),
        }
    }
}

impl RepoAdmin {
    fn get_all(&self, _: Request<Body>) -> FutureResponse {
        #[derive(Serialize)]
        struct ReposResp {
            repos: Vec<RepoInfo>,
        }

        let repos = match self.config.repos().get_all() {
            Ok(u) => u,
            Err(e) => {
                return self.respond_error(&format!("{}", e));
            }
        };
        let resp = ReposResp { repos: repos };

        let repos = match serde_json::to_string(&resp) {
            Ok(u) => u,
            Err(e) => {
                error!("Error serializing repos: {}", e);
                String::new()
            }
        };
        self.respond(util::new_json_resp(repos))
    }

    fn create(&self, req: Request<Body>) -> FutureResponse {
        let config = self.config.clone();
        parse_json(req, move |repo: RepoInfo| {
            if let Err(e) = config.repos_write().insert_info(&repo) {
                error!("{}", e);
                return util::new_empty_error_resp();
            }
            util::new_empty_resp(StatusCode::OK)
        })
    }

    fn update(&self, req: Request<Body>) -> FutureResponse {
        let config = self.config.clone();

        parse_json(req, move |repo: RepoInfo| {
            if let Err(e) = config.repos_write().update(&repo) {
                error!("{}", e);
                return util::new_empty_error_resp();
            }
            util::new_empty_resp(StatusCode::OK)
        })
    }

    fn delete(&self, req: Request<Body>) -> FutureResponse {
        let config = self.config.clone();

        let query = util::parse_query(req.uri().query());

        let repo_id = match query.get("id").map(|id| id.parse::<i32>()) {
            None | Some(Err(_)) => return self.respond(util::new_bad_req_resp("No `id` param specified")),
            Some(Ok(id)) => id,
        };

        if let Err(e) = config.repos_write().delete(repo_id) {
            return self.respond_error(&format!("{}", e));
        }
        self.respond_with(StatusCode::OK, "")
    }
}

pub struct MergeVersions {
    config: Arc<Config>,
}

impl MergeVersions {
    pub fn new(config: Arc<Config>) -> Box<MergeVersions> {
        Box::new(MergeVersions { config: config })
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
    fn handle(&self, req: Request<Body>) -> FutureResponse {
        let config = self.config.clone();
        parse_json(req, move |merge_req: MergeVersionsReq| {
            // make a copy of the jira config so we can modify the auth
            let mut jira_config: JiraConfig = match config.jira {
                Some(ref j) => j.clone(),
                None => return util::new_bad_req_resp("No JIRA config"),
            };

            if !merge_req.dry_run {
                jira_config.username = merge_req.admin_user.unwrap_or(String::new());
                jira_config.password = merge_req.admin_pass.unwrap_or(String::new());

                if jira_config.username.is_empty() || jira_config.password.is_empty() {
                    return util::new_bad_req_resp("JIRA auth required for non dry-run");
                }
            }

            let jira_sess = match jira::api::JiraSession::new(&jira_config) {
                Ok(j) => j,
                Err(e) => return util::new_bad_req_resp(format!("Error creating JIRA session: {}", e)),
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
                    return util::new_error_resp(format!("Error merging pending versions: {}", e));
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
                    return util::new_error_resp(format!("Error serializing pending versions: {}", e));
                }
            };

            util::new_json_resp(resp_json)
        })
    }
}
