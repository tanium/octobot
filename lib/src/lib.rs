#![allow(clippy::new_without_default)]

pub mod config;
pub mod config_db;
mod config_db_migrations;
pub mod db;
pub mod github;
pub mod http_client;
pub mod jira;
pub mod jwt;
pub mod metrics;
pub mod passwd;
pub mod repos;
pub mod slack;
pub mod users;
pub mod version;

pub mod errors {
    pub type Error = anyhow::Error;
    pub type Result<T> = anyhow::Result<T>;
}
