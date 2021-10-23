#[allow(clippy::module_inception)]
mod db;
pub mod migrations;

pub use self::db::*;
