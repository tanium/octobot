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

use config::Config;
use users::UserHostMap;
use repos::RepoHostMap;


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
    fn handle(&self, req: &mut Request) -> IronResult<Response> {
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
    fn handle(&self, req: &mut Request) -> IronResult<Response> {
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
        // TODO: centralize this... :(
        let body = match req.get::<bodyparser::Raw>() {
            Ok(Some(j)) => j,
            Err(_) | Ok(None) => {
                error!("Error reading json");
                return Ok(Response::with((status::BadRequest, format!("Error reading json"))));
            }
        };
        let users: UserHostMap = match serde_json::from_str(&body) {
            Ok(r) => r,
            Err(e) => {
                error!("Error parsing update users request: {}", e);
                return Ok(Response::with((status::BadRequest,
                                          format!("Error parsing JSON: {}", e))));
            }
        };

        let json = match serde_json::to_string_pretty(&users) {
            Ok(j) => j,
            Err(e) => {
                error!("Error serializing users: {}", e);
                return Ok(Response::with((status::BadRequest, format!("Error serializing JSON: {}", e))));
            }
        };

        let config_file = self.config.users_config_file.clone();
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
        // TODO: centralize this... :(
        let body = match req.get::<bodyparser::Raw>() {
            Ok(Some(j)) => j,
            Err(_) | Ok(None) => {
                error!("Error reading json");
                return Ok(Response::with((status::BadRequest, format!("Error reading json"))));
            }
        };
        let repos: RepoHostMap = match serde_json::from_str(&body) {
            Ok(r) => r,
            Err(e) => {
                error!("Error parsing update repos request: {}", e);
                return Ok(Response::with((status::BadRequest,
                                          format!("Error parsing JSON: {}", e))));
            }
        };

        let json = match serde_json::to_string_pretty(&repos) {
            Ok(j) => j,
            Err(e) => {
                error!("Error serializing repos: {}", e);
                return Ok(Response::with((status::BadRequest, format!("Error serializing JSON: {}", e))));
            }
        };


        let config_file = self.config.repos_config_file.clone();
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

