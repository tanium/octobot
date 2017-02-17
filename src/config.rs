use std::fs;
use std::io::Read;
use toml;

use users;
use repos;

#[derive(Clone)]
pub struct Config {
    pub slack_webhook_url: String,
    pub github_secret: String,
    pub listen_addr: Option<String>,
    pub clone_root_dir: String,
    pub github_host: String,
    pub github_token: String,
    pub users: users::UserConfig,
    pub repos: repos::RepoConfig,
    pub jira: Option<JiraConfig>,
}

#[derive(Deserialize, Debug)]
pub struct ConfigModel {
    pub main: MainConfig,
    pub github: GithubConfig,
    pub jira: Option<JiraConfig>,
}

#[derive(Deserialize, Debug)]
pub struct MainConfig {
    pub slack_webhook_url: String,
    pub listen_addr: Option<String>,
    pub users_config_file: String,
    pub repos_config_file: String,
    pub clone_root_dir: String,
}

#[derive(Deserialize, Debug)]
pub struct GithubConfig {
    pub webhook_secret: String,
    pub host: String,
    pub api_token: String,
}

#[derive(Deserialize, Clone, Debug)]
pub struct JiraConfig {
    pub host: String,
    pub username: String,
    pub password: String,

    // review state that may be necessary before submitting for review (defaults to ["In Progress"])
    pub progress_states: Option<Vec<String>>,
    // review state to transition to when marked for review (defaults to ["Pending Review"])
    pub review_states: Option<Vec<String>>,
    // resolved state to transition to when PR is merged. (defaults to ["Resolved", "Done"])
    pub resolved_states: Option<Vec<String>>,
    // when marking as resolved, add this resolution (defaults to ["Fixed", "Done"])
    pub fixed_resolutions: Option<Vec<String>>,
}

impl Config {
    pub fn empty_config() -> Config {
        Config::new(users::UserConfig::new(), repos::RepoConfig::new())
    }

    pub fn new(users: users::UserConfig, repos: repos::RepoConfig) -> Config {
        Config::new_with_model(ConfigModel::new(), users, repos)
    }

    pub fn new_with_model(config: ConfigModel, users: users::UserConfig, repos: repos::RepoConfig) -> Config {
        Config {
            slack_webhook_url: config.main.slack_webhook_url,
            github_secret: config.github.webhook_secret,
            listen_addr: config.main.listen_addr,
            github_host: config.github.host,
            github_token: config.github.api_token,
            clone_root_dir: config.main.clone_root_dir,
            users: users,
            repos: repos,
            jira: config.jira,
        }
    }
}

impl ConfigModel {
    pub fn new() -> ConfigModel {
        ConfigModel {
            main: MainConfig {
                slack_webhook_url: String::new(),
                listen_addr: None,
                users_config_file: String::new(),
                repos_config_file: String::new(),
                clone_root_dir: String::new(),
            },
            github: GithubConfig {
                webhook_secret: String::new(),
                host: String::new(),
                api_token: String::new(),
            },
            jira: None,
        }
    }
}

impl JiraConfig {
    pub fn progress_states(&self) -> Vec<String> {
        if let Some(ref states) = self.progress_states {
            states.clone() // hmm. do these w/o a clone?
        } else {
            vec!["In Progress".into()]
        }
    }

    pub fn review_states(&self) -> Vec<String> {
        if let Some(ref states) = self.review_states {
            states.clone() // hmm. do these w/o a clone?
        } else {
            vec!["Pending Review".into()]
        }
    }

    pub fn resolved_states(&self) -> Vec<String> {
        if let Some(ref states) = self.resolved_states {
            states.clone() // hmm. do these w/o a clone?
        } else {
            vec!["Resolved".into(), "Done".into()]
        }
    }

    pub fn fixed_resolutions(&self) -> Vec<String> {
        if let Some(ref res) = self.fixed_resolutions {
            res.clone() // hmm. do these w/o a clone?
        } else {
            vec!["Fixed".into(), "Done".into()]
        }
    }
}

pub fn parse(config_file: String) -> Result<Config, String> {
    let mut config_file_open = match fs::File::open(config_file.clone()) {
        Ok(f) => f,
        Err(e) => return Err(format!("Could not open config file '{}': {}", config_file, e)),
    };
    let mut config_contents = String::new();
    match config_file_open.read_to_string(&mut config_contents) {
        Ok(_) => (),
        Err(e) => return Err(format!("Could not read config file '{}': {}", config_file, e)),
    };
    parse_string_and_load(&config_contents)
}

fn parse_string(config_contents: &str) -> Result<ConfigModel, String> {
    match toml::from_str::<ConfigModel>(config_contents) {
        Ok(c) => Ok(c),
        Err(e) => return Err(format!("Error decoding config: {}", e)),
    }
}

fn parse_string_and_load(config_contents: &str) -> Result<Config, String> {
    let config = try!(parse_string(config_contents));

    let users = match users::load_config(&config.main.users_config_file) {
        Ok(c) => c,
        Err(e) => return Err(format!("Error reading user config file: {}", e)),
    };

    let repos = match repos::load_config(&config.main.repos_config_file) {
        Ok(c) => c,
        Err(e) => return Err(format!("Error reading repo config file: {}", e)),
    };

    Ok(Config::new_with_model(config, users, repos))
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse() {
        let config_str = r#"
[main]
slack_webhook_url = "https://hooks.slack.com/foo"
users_config_file = "users.json"
repos_config_file = "repos.json"
clone_root_dir = "./repos"

[github]
webhook_secret = "abcd"
host = "git.company.com"
api_token = "some-tokens"
"#;
        let config = parse_string(config_str).unwrap();

        assert_eq!("https://hooks.slack.com/foo", config.main.slack_webhook_url);
        assert_eq!("users.json", config.main.users_config_file);
        assert_eq!("repos.json", config.main.repos_config_file);

    }
}
