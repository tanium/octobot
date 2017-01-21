use std::fs;
use std::io::Read;
use super::toml;

use super::users;
use super::repos;

pub struct Config {
    pub slack_webhook_url: String,
    pub github_secret: String,
    pub listen_addr: Option<String>,
    pub github_host: String,
    pub github_token: String,
    pub clone_root_dir: String,
    pub users: users::UserConfig,
    pub repos: repos::RepoConfig,
}


#[derive(RustcDecodable, Debug)]
pub struct ConfigModel {
    pub slack_webhook_url: String,
    pub github_secret: String,
    pub listen_addr: Option<String>,
    pub users_config_file: String,
    pub repos_config_file: String,
    pub github_host: String,
    pub github_token: String,
    pub clone_root_dir: String,
}

impl Config {
    pub fn new(users: users::UserConfig, repos: repos::RepoConfig) -> Config {
        Config::new_with_model(ConfigModel::new(), users, repos)
    }

    pub fn new_with_model(config: ConfigModel, users: users::UserConfig, repos: repos::RepoConfig) -> Config {
        Config {
            slack_webhook_url: config.slack_webhook_url,
            github_secret: config.github_secret,
            listen_addr: config.listen_addr,
            github_host: config.github_host,
            github_token: config.github_token,
            clone_root_dir: config.clone_root_dir,
            users: users,
            repos: repos,
        }
    }
}

impl ConfigModel {
    pub fn new() -> ConfigModel {
        ConfigModel {
            slack_webhook_url: String::new(),
            github_secret: String::new(),
            listen_addr: None,
            users_config_file: String::new(),
            repos_config_file: String::new(),
            github_host: String::new(),
            github_token: String::new(),
            clone_root_dir: String::new(),
        }
    }
}

pub fn parse(config_file: String) -> Result<Config, String> {
    let mut config_file_open = match fs::File::open(config_file.clone()) {
        Ok(f) => f,
        Err(e) => return Err(format!("Could not open backup file '{}': {}", config_file, e)),
    };
    let mut config_contents = String::new();
    match config_file_open.read_to_string(&mut config_contents) {
        Ok(_) => (),
        Err(e) => return Err(format!("Could not read config file '{}': {}", config_file, e)),
    };

    let config_value = match toml::Parser::new(config_contents.as_str()).parse() {
        Some(c) => c,
        None => return Err(format!("Could not decode config file '{}'", config_file)),
    };

    let config: ConfigModel = match config_value.get("config") {
        Some(c1) => {
            match toml::decode(c1.clone()) {
                Some(c2) => c2,
                None => return Err(format!("Error decoding to config object")),
            }
        }
        None => return Err(format!("No config section found")),
    };

    let users = match users::load_config(&config.users_config_file) {
        Ok(c) => c,
        Err(e) => return Err(format!("Error reading user config file: {}", e)),
    };

    let repos = match repos::load_config(&config.repos_config_file) {
        Ok(c) => c,
        Err(e) => return Err(format!("Error reading repo config file: {}", e)),
    };

    Ok(Config::new_with_model(config, users, repos))
}

