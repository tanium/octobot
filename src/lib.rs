extern crate base64;
extern crate bodyparser;
extern crate logger;
extern crate futures;
extern crate hyper;
extern crate hyper_rustls;
extern crate iron;
#[macro_use]
extern crate maplit;
extern crate regex;
extern crate ring;
#[macro_use]
extern crate router;
extern crate rustc_serialize;
extern crate serde;
#[macro_use]
extern crate serde_json;
extern crate threadpool;
extern crate tokio_core;
extern crate toml;
extern crate unidiff;
extern crate url;

#[macro_use]
extern crate log;
#[macro_use]
extern crate serde_derive;

pub mod config;
pub mod diffs;
pub mod dir_pool;
pub mod force_push;
pub mod git;
pub mod git_clone_manager;
pub mod github;
pub mod http_client;
pub mod jira;
pub mod messenger;
pub mod pr_merge;
pub mod repos;
pub mod repo_version;
pub mod server;
pub mod slack;
pub mod users;
pub mod util;
pub mod version;
pub mod worker;

#[cfg(target_os="linux")]
mod docker;
