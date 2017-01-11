use super::std::fs;
use super::std::io::{Read};
use super::toml;

#[derive(RustcDecodable, Debug)]
pub struct Config {
    pub slack_webhook_url: String,
    pub github_secret: String,
    pub listen_addr: Option<String>,
    pub users_config_file: String,
    pub repos_config_file: String,
}

pub fn parse(config_file: String) -> Result<Config, String> {
    let mut config_file_open = match fs::File::open(config_file.clone()) {
        Ok(f) => f,
        Err(e) => return Err(format!("Could not open backup file '{}': {}", config_file, e))
    };
    let mut config_contents = String::new();
    match config_file_open.read_to_string(&mut config_contents) {
        Ok(_) => (),
        Err(e) => return Err(format!("Could not read config file '{}': {}", config_file, e))
    };

    let config_value = match toml::Parser::new(config_contents.as_str()).parse() {
        Some(c) => c,
        None => return Err(format!("Could not decode config file '{}'", config_file))
    };

    let config: Config = match config_value.get("config") {
        Some(c1) => match toml::decode(c1.clone()) {
            Some(c2) => c2,
            None => return Err(format!("Error decoding to config object"))
        },
        None => return Err(format!("No config section found"))
    };

    Ok(config)
}
