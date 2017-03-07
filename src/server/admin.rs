use std::sync::Arc;

use bodyparser;
use iron::prelude::*;
use iron::status;
use iron::headers::ContentType;
use iron::middleware::Handler;
use iron::modifiers::Header;
use serde_json;

use config::Config;


pub struct GetUsers {
    config: Arc<Config>
}

pub struct GetRepos {
    config: Arc<Config>
}

impl GetUsers {
    pub fn new(config: Arc<Config>) -> GetUsers {
        GetUsers {
            config: config,
        }
    }
}

impl GetRepos {
    pub fn new(config: Arc<Config>) -> GetRepos {
        GetRepos {
            config: config
        }
    }
}

impl Handler for GetUsers {
    fn handle(&self, req: &mut Request) -> IronResult<Response> {
        let users = match serde_json::to_string(&self.config.users) {
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
    fn handle(&self, req: &mut Request) -> IronResult<Response> {
        let repos = match serde_json::to_string(&self.config.repos) {
            Ok(u) => u,
            Err(e) => {
                error!("Error serializing repos: {}", e);
                String::new()
            }
        };
        Ok(Response::with((status::Ok, Header(ContentType::json()), repos)))
    }
}

