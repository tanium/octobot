pub mod config;
pub mod db;
pub mod diffs;
pub mod dir_pool;
pub mod force_push;
pub mod git;
pub mod git_clone_manager;
pub mod github;
pub mod http_client;
pub mod ldap_auth;
pub mod jira;
pub mod jwt;
pub mod messenger;
pub mod pr_merge;
pub mod repos;
pub mod repo_version;
pub mod runtime;
pub mod server;
pub mod slack;
pub mod users;
pub mod util;
pub mod version;
pub mod worker;

pub mod errors {
    pub type Error = failure::Error;
    pub type Result<T> = std::result::Result<T, failure::Error>;
}

#[cfg(target_os = "linux")]
mod docker;
