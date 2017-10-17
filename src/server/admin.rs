use std::borrow::Borrow;
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::sync::Arc;

use bodyparser;
use iron::prelude::*;
use iron::status;
use iron::headers::ContentType;
use iron::middleware::Handler;
use iron::modifiers::Header;
use serde_json;

use config::{Config, JiraConfig};
use jira;
use users::UserHostMap;
use repos::RepoHostMap;
use version;

pub struct GetUsers {
    config: Arc<Config>
}

pub struct UpdateUsers {
    config: Arc<Config>
}

pub struct GetRepos {
    config: Arc<Config>
}

pub struct UpdateRepos {
    config: Arc<Config>
}

impl GetUsers {
    pub fn new(config: Arc<Config>) -> GetUsers {
        GetUsers { config: config }
    }
}

impl UpdateUsers {
    pub fn new(config: Arc<Config>) -> UpdateUsers {
        UpdateUsers { config: config }
    }
}

impl GetRepos {
    pub fn new(config: Arc<Config>) -> GetRepos {
        GetRepos { config: config }
    }
}

impl UpdateRepos {
    pub fn new(config: Arc<Config>) -> UpdateRepos {
        UpdateRepos { config: config }
    }
}

impl Handler for GetUsers {
    fn handle(&self, _: &mut Request) -> IronResult<Response> {
        let users = match serde_json::to_string(&*self.config.users()) {
            Ok(u) => u,
            Err(e) => {
                error!("Error serializing users: {}", e);
                String::new()
            }
        };
        Ok(Response::with((status::Ok, Header(ContentType::json()), users)))

    }
}

impl Handler for GetRepos {
    fn handle(&self, _: &mut Request) -> IronResult<Response> {
        let repos = match serde_json::to_string(&*self.config.repos()) {
            Ok(u) => u,
            Err(e) => {
                error!("Error serializing repos: {}", e);
                String::new()
            }
        };
        Ok(Response::with((status::Ok, Header(ContentType::json()), repos)))
    }
}

impl Handler for UpdateUsers {
    fn handle(&self, req: &mut Request) -> IronResult<Response> {
        let users = match req.get::<bodyparser::Struct<UserHostMap>>() {
            Ok(Some(j)) => j,
            Err(_) | Ok(None) => {
                error!("Error reading json");
                return Ok(Response::with((status::BadRequest, format!("Error reading json"))));
            }
        };

        let json = match serde_json::to_string_pretty(&users) {
            Ok(j) => j,
            Err(e) => {
                error!("Error serializing users: {}", e);
                return Ok(Response::with((status::BadRequest, format!("Error serializing JSON: {}", e))));
            }
        };

        let config_file = self.config.main.users_config_file.clone();
        let config_file_tmp = config_file.clone() + ".tmp";

        let mut file = match fs::File::create(&config_file_tmp) {
            Ok(f) => f,
            Err(e) => {
                error!("Error opening file: {}", e);
                return Ok(Response::with((status::InternalServerError, format!("Error writing file: {}", e))));
            }
        };

        if let Err(e) = file.write_all(json.as_bytes()) {
            error!("Error writing file: {}", e);
            return Ok(Response::with((status::InternalServerError, format!("Error writing file: {}", e))));
        }

        if let Err(e) = fs::rename(&config_file_tmp, &config_file) {
            error!("Error renaming file: {}", e);
            return Ok(Response::with((status::InternalServerError, format!("Error renaming file: {}", e))));
        }

        if let Err(e) = self.config.reload_users_repos() {
            error!("Error reloading config: {}", e);
        }

        Ok(Response::with((status::Ok)))
    }
}

impl Handler for UpdateRepos {
    fn handle(&self, req: &mut Request) -> IronResult<Response> {
        let repos = match req.get::<bodyparser::Struct<RepoHostMap>>() {
            Ok(Some(j)) => j,
            Err(_) | Ok(None) => {
                error!("Error reading json");
                return Ok(Response::with((status::BadRequest, format!("Error reading json"))));
            }
        };

        let json = match serde_json::to_string_pretty(&repos) {
            Ok(j) => j,
            Err(e) => {
                error!("Error serializing repos: {}", e);
                return Ok(Response::with((status::BadRequest, format!("Error serializing JSON: {}", e))));
            }
        };

        let config_file = self.config.main.repos_config_file.clone();
        let config_file_tmp = config_file.clone() + ".tmp";

        let mut file = match fs::File::create(&config_file_tmp) {
            Ok(f) => f,
            Err(e) => {
                error!("Error opening file: {}", e);
                return Ok(Response::with((status::InternalServerError, format!("Error writing file: {}", e))));
            }
        };

        if let Err(e) = file.write_all(json.as_bytes()) {
            error!("Error writing file: {}", e);
            return Ok(Response::with((status::InternalServerError, format!("Error writing file: {}", e))));
        }

        if let Err(e) = fs::rename(&config_file_tmp, &config_file) {
            error!("Error renaming file: {}", e);
            return Ok(Response::with((status::InternalServerError, format!("Error renaming file: {}", e))));
        }

        if let Err(e) = self.config.reload_users_repos() {
            error!("Error reloading config: {}", e);
        }

        Ok(Response::with((status::Ok)))
    }
}

pub struct MergeVersions {
    config: Arc<Config>,
}

impl MergeVersions {
    pub fn new(config: Arc<Config>) -> MergeVersions {
        MergeVersions {
            config: config,
        }
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
    fn handle(&self, req: &mut Request) -> IronResult<Response> {
        let merge_req = match req.get::<bodyparser::Struct<MergeVersionsReq>>() {
            Ok(Some(j)) => j,
            Err(_) | Ok(None) => {
                error!("Error reading json");
                return Ok(Response::with((status::BadRequest, format!("Error reading json"))));
            }
        };

        // make a copy of the jira config so we can modify the auth
        let mut jira_config: JiraConfig = match self.config.jira {
            Some(ref j) => j.clone(),
            None => {
                return Ok(Response::with((status::BadRequest, format!("No JIRA config"))));
            }
        };

        if !merge_req.dry_run {
            jira_config.username = merge_req.admin_user.unwrap_or(String::new());
            jira_config.password = merge_req.admin_pass.unwrap_or(String::new());

            if jira_config.username.is_empty() || jira_config.password.is_empty() {
                return Ok(Response::with((status::BadRequest, format!("JIRA auth required for non dry-run"))));
            }
        }

        let jira_sess = match jira::api::JiraSession::new(&jira_config) {
            Ok(j) => j,
            Err(e) => {
                return Ok(Response::with((status::BadRequest, format!("Error creating JIRA session: {}", e))));
            }
        };

        let dry_run_mode = if merge_req.dry_run {
            jira::workflow::DryRunMode::DryRun
        } else {
            jira::workflow::DryRunMode::ForReal
        };

        let all_relevant_versions = match jira::workflow::merge_pending_versions(&merge_req.version, &merge_req.project, jira_sess.borrow(), dry_run_mode) {
            Ok(v) => v,
            Err(e) => {
                error!("Error merging pending versions: {}", e);
                return Ok(Response::with((status::InternalServerError, format!("Error merging pending versions"))));
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
                return Ok(Response::with((status::InternalServerError, format!("Error serializing pending versions"))));
            }
        };
        Ok(Response::with((status::Ok, Header(ContentType::json()), resp_json)))
    }
}
