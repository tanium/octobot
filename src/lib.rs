// `error_chain!` can recurse deeply
#![recursion_limit = "1024"]

extern crate base64;
#[macro_use]
extern crate error_chain;
extern crate futures;
extern crate http;
extern crate hyper;
extern crate hyper_rustls;
extern crate jsonwebtoken;
#[macro_use]
extern crate maplit;
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

#[macro_use]
extern crate log;
#[macro_use]
extern crate serde_derive;

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
    // Create the Error, ErrorKind, ResultExt, and Result types
    error_chain!{
        foreign_links {
            Fmt(::std::fmt::Error);
            Io(::std::io::Error);
            Url(::url::ParseError);
            DB(::rusqlite::Error);
            LDAP(::openldap::errors::LDAPError);
        }
    }
}

mod db_migrations;
#[cfg(target_os = "linux")]
mod docker;
