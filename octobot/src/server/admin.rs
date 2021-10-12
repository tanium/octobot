use std::borrow::Borrow;
use std::collections::HashMap;
use std::sync::Arc;

use hyper::StatusCode;
use hyper::{Body, Request, Response};
use log::error;
use serde_derive::{Deserialize, Serialize};
use serde_json;

use octobot_lib::config::{Config, JiraConfig};
use octobot_lib::errors::*;
use octobot_lib::jira;
use octobot_lib::repos::RepoInfo;
use octobot_lib::users::UserInfo;
use octobot_lib::version;
use octobot_ops::util;

use crate::http_util;
use crate::server::http::{parse_json, Handler};

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
            config,
            op,
        })
    }
}

impl RepoAdmin {
    pub fn new(config: Arc<Config>, op: Op) -> Box<RepoAdmin> {
        Box::new(RepoAdmin {
            config,
            op,
        })
    }
}

#[async_trait::async_trait]
impl Handler for UserAdmin {
    async fn handle(&self, req: Request<Body>) -> Result<Response<Body>> {
        match &self.op {
            &Op::List => self.get_all(req).await,
            &Op::Create => self.create(req).await,
            &Op::Update => self.update(req).await,
            &Op::Delete => self.delete(req).await,
        }
    }
}

impl UserAdmin {
    async fn get_all(&self, _: Request<Body>) -> Result<Response<Body>> {
        #[derive(Serialize)]
        struct UsersResp {
            users: Vec<UserInfo>,
        }

        let users = self.config.users().get_all()?;
        let resp = UsersResp { users };

        let users = serde_json::to_string(&resp)?;

        Ok(http_util::new_json_resp(users))
    }

    async fn create(&self, req: Request<Body>) -> Result<Response<Body>> {
        let config = self.config.clone();
        let user: UserInfo = parse_json(req).await?;
        config.users_write().insert_info(&user)?;

        Ok(http_util::new_empty_resp(StatusCode::OK))
    }

    async fn update(&self, req: Request<Body>) -> Result<Response<Body>> {
        let config = self.config.clone();

        let user: UserInfo = parse_json(req).await?;
        config.users_write().update(&user)?;

        Ok(http_util::new_empty_resp(StatusCode::OK))
    }

    async fn delete(&self, req: Request<Body>) -> Result<Response<Body>> {
        let config = self.config.clone();

        let query = util::parse_query(req.uri().query());

        let user_id = match query.get("id").map(|id| id.parse::<i32>()) {
            None | Some(Err(_)) => {
                return Ok(http_util::new_bad_req_resp("No `id` param specified"))
            }
            Some(Ok(id)) => id,
        };

        if let Err(e) = config.users_write().delete(user_id) {
            return Ok(self.respond_error(&format!("{}", e)));
        }

        Ok(http_util::new_empty_resp(StatusCode::OK))
    }
}

#[async_trait::async_trait]
impl Handler for RepoAdmin {
    async fn handle(&self, req: Request<Body>) -> Result<Response<Body>> {
        match &self.op {
            &Op::List => self.get_all(req).await,
            &Op::Create => self.create(req).await,
            &Op::Update => self.update(req).await,
            &Op::Delete => self.delete(req).await,
        }
    }
}

impl RepoAdmin {
    async fn get_all(&self, _: Request<Body>) -> Result<Response<Body>> {
        #[derive(Serialize)]
        struct ReposResp {
            repos: Vec<RepoInfo>,
        }

        let repos = match self.config.repos().get_all() {
            Ok(u) => u,
            Err(e) => {
                return Ok(self.respond_error(&format!("{}", e)));
            }
        };
        let resp = ReposResp { repos };

        let repos = match serde_json::to_string(&resp) {
            Ok(u) => u,
            Err(e) => {
                error!("Error serializing repos: {}", e);
                String::new()
            }
        };

        Ok(http_util::new_json_resp(repos))
    }

    async fn create(&self, req: Request<Body>) -> Result<Response<Body>> {
        let config = self.config.clone();
        let repo: RepoInfo = parse_json(req).await?;
        config.repos_write().insert_info(&repo)?;

        Ok(http_util::new_empty_resp(StatusCode::OK))
    }

    async fn update(&self, req: Request<Body>) -> Result<Response<Body>> {
        let config = self.config.clone();
        let repo: RepoInfo = parse_json(req).await?;
        config.repos_write().update(&repo)?;

        Ok(http_util::new_empty_resp(StatusCode::OK))
    }

    async fn delete(&self, req: Request<Body>) -> Result<Response<Body>> {
        let config = self.config.clone();

        let query = util::parse_query(req.uri().query());

        let repo_id = match query.get("id").map(|id| id.parse::<i32>()) {
            None | Some(Err(_)) => {
                return Ok(http_util::new_bad_req_resp("No `id` param specified"))
            }
            Some(Ok(id)) => id,
        };

        if let Err(e) = config.repos_write().delete(repo_id) {
            return Ok(self.respond_error(&format!("{}", e)));
        }

        Ok(self.respond_with(StatusCode::OK, ""))
    }
}

pub struct MergeVersions {
    config: Arc<Config>,
}

impl MergeVersions {
    pub fn new(config: Arc<Config>) -> Box<MergeVersions> {
        Box::new(MergeVersions { config })
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
    login_suffix: Option<String>,
    versions: HashMap<String, Vec<version::Version>>,
}

#[async_trait::async_trait]
impl Handler for MergeVersions {
    async fn handle(&self, req: Request<Body>) -> Result<Response<Body>> {
        let merge_req: MergeVersionsReq = parse_json(req).await?;

        Ok(self.do_handle(merge_req).await)
    }
}

impl MergeVersions {
    // TODO: This returns error codes as HTTP 400, so we want to not hide the error message
    async fn do_handle(&self, merge_req: MergeVersionsReq) -> Response<Body> {
        let config = self.config.clone();
        // make a copy of the jira config so we can modify the auth
        let mut jira_config: JiraConfig = match config.jira {
            Some(ref j) => j.clone(),
            None => return http_util::new_bad_req_resp("No JIRA config"),
        };

        if !merge_req.dry_run {
            jira_config.username = merge_req.admin_user.unwrap_or_default();
            jira_config.password = merge_req.admin_pass.unwrap_or_default();

            if jira_config.username.is_empty() || jira_config.password.is_empty() {
                return http_util::new_bad_req_resp("JIRA auth required for non dry-run");
            }
        }

        let jira_sess = match jira::api::JiraSession::new(&jira_config, None).await {
            Ok(j) => j,
            Err(e) => {
                return http_util::new_bad_req_resp(format!("Error creating JIRA session: {}", e))
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
        )
        .await
        {
            Ok(v) => v,
            Err(e) => {
                error!("Error merging pending versions: {}", e);
                return http_util::new_error_resp(format!("Error merging pending versions: {}", e));
            }
        };

        if !merge_req.dry_run {
            if let Err(e) =
                jira::workflow::sort_versions(&merge_req.project, jira_sess.borrow()).await
            {
                error!("Error sorting versions: {}", e);
            }
        }

        let resp = MergeVersionsResp {
            jira_base: jira_config.base_url(),
            login_suffix: jira_config.login_suffix,
            versions: all_relevant_versions,
        };

        let resp_json = match serde_json::to_string(&resp) {
            Ok(r) => r,
            Err(e) => {
                error!("Error serializing versions: {}", e);
                return http_util::new_error_resp(format!(
                    "Error serializing pending versions: {}",
                    e
                ));
            }
        };

        http_util::new_json_resp(resp_json)
    }
}
