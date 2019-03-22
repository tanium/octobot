extern crate base64;
#[macro_use]
extern crate failure;
extern crate futures;
extern crate http;
extern crate hyper;
extern crate hyper_rustls;
extern crate jsonwebtoken;
extern crate openldap;
extern crate regex;
extern crate ring;
extern crate rustc_serialize;
extern crate rustls;
extern crate serde;
#[macro_use]
extern crate serde_json;
extern crate tokio;
extern crate tokio_rustls;
extern crate tokio_threadpool;
extern crate toml;
extern crate unidiff;
extern crate url;
extern crate time;
extern crate rusqlite;
extern crate reqwest;

#[macro_use]
extern crate log;
#[macro_use]
extern crate serde_derive;

#[cfg(test)]
#[macro_use]
extern crate maplit;

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

mod db_migrations;
#[cfg(target_os = "linux")]
mod docker;
