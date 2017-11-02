use std::fs;
use std::io::{Read, Write};
use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};
use toml;

use repos;
use users;

pub struct Config {
    pub main: MainConfig,
    pub admin: Option<AdminConfig>,
    pub github: GithubConfig,
    pub jira: Option<JiraConfig>,
    pub ldap: Option<LdapConfig>,

    pub users: RwLock<users::UserConfig>,
    pub repos: RwLock<repos::RepoConfig>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ConfigModel {
    pub main: MainConfig,
    pub admin: Option<AdminConfig>,
    pub github: GithubConfig,
    pub jira: Option<JiraConfig>,
    pub ldap: Option<LdapConfig>
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MainConfig {
    pub slack_webhook_url: String,
    pub listen_addr: Option<String>,
    pub listen_addr_ssl: Option<String>,
    pub users_config_file: String,
    pub repos_config_file: String,
    pub clone_root_dir: String,
    pub ssl_cert_file: Option<String>,
    pub ssl_key_file: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AdminConfig {
    pub name: String,
    pub salt: String,
    pub pass_hash: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct GithubConfig {
    pub webhook_secret: String,
    pub host: String,
    pub api_token: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
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
    // the field name for where the version goes. (defaults to "fixVersions").
    pub fix_versions_field: Option<String>,
    // the field name for where the pending build versions go. expected to be a plain text field
    pub pending_versions_field: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LdapConfig {
    // LDAP URL (e.g. ldaps://ldap.company.com)
    pub url: String,
    // either username for AD or bind DN for LDAP
    pub bind_user: String,
    // bind user's password
    pub bind_pass: String,
    // the base DN to bind to
    pub base_dn: String,
    // attribute to match logins against (e.g. samAccountName for AD, uid for LDAP)
    pub userid_attribute: String,
    // Additional LDAP search filter for user types and group membership
    // e.g. (&(objectCategory=Person)(memberOf=cn=octobot-admins,ou=users,dc=company,dc=com))
    pub search_filter: Option<String>,
}

impl Config {
    pub fn empty_config() -> Config {
        Config::new(users::UserConfig::new(), repos::RepoConfig::new())
    }

    pub fn new(users: users::UserConfig, repos: repos::RepoConfig) -> Config {
        Config::new_with_model(ConfigModel::new(), users, repos)
    }

    fn new_with_model(config: ConfigModel, users: users::UserConfig, repos: repos::RepoConfig) -> Config {
        Config {
            main: config.main,
            admin: config.admin,
            github: config.github,
            jira: config.jira,
            ldap: config.ldap,
            users: RwLock::new(users),
            repos: RwLock::new(repos),
        }
    }

    pub fn save(&self, config_file: &str) -> Result<(), String> {
        let model = ConfigModel {
            main: self.main.clone(),
            admin: self.admin.clone(),
            github: self.github.clone(),
            jira: self.jira.clone(),
            ldap: self.ldap.clone(),
        };

        let serialized = match toml::to_string(&model) {
            Ok(c) => c,
            Err(e) => return Err(format!("Error serializing config: {}", e)),
        };

        let tmp_file = config_file.to_string() + ".tmp";
        let bak_file = config_file.to_string() + ".bak";

        let mut file = match fs::File::create(&tmp_file) {
            Ok(f) => f,
            Err(e) => return Err(format!("Error opening file: {}", e)),
        };

        if let Err(e) = file.write_all(serialized.as_bytes()) {
            return Err(format!("Error writing file: {}", e));
        }

        if let Err(e) = fs::rename(&config_file, &bak_file) {
            return Err(format!("Error backing up config file: {}", e));
        }

        if let Err(e) = fs::rename(&tmp_file, &config_file) {
            return Err(format!("Error renaming temp file: {}", e));
        }

        if let Err(e) = fs::remove_file(&bak_file) {
            info!("Error removing backup file: {}", e);
        }

        Ok(())
    }

    pub fn reload_users_repos(&self) -> Result<(), String> {
        let users = match users::load_config(&self.main.users_config_file) {
            Ok(c) => c,
            Err(e) => return Err(format!("Error reading user config file: {}", e)),
        };

        let repos = match repos::load_config(&self.main.repos_config_file) {
            Ok(c) => c,
            Err(e) => return Err(format!("Error reading repo config file: {}", e)),
        };

        *self.users.write().unwrap() = users;
        *self.repos.write().unwrap() = repos;
        Ok(())
    }

    pub fn users(&self) -> RwLockReadGuard<users::UserConfig> {
        self.users.read().unwrap()
    }

    pub fn users_write(&self) -> RwLockWriteGuard<users::UserConfig> {
        self.users.write().unwrap()
    }

    pub fn repos(&self) -> RwLockReadGuard<repos::RepoConfig> {
        self.repos.read().unwrap()
    }

    pub fn repos_write(&self) -> RwLockWriteGuard<repos::RepoConfig> {
        self.repos.write().unwrap()
    }
}

impl ConfigModel {
    pub fn new() -> ConfigModel {
        ConfigModel {
            main: MainConfig {
                slack_webhook_url: String::new(),
                listen_addr: None,
                listen_addr_ssl: None,
                users_config_file: String::new(),
                repos_config_file: String::new(),
                clone_root_dir: String::new(),
                ssl_cert_file: None,
                ssl_key_file: None,
            },
            admin: None,
            github: GithubConfig {
                webhook_secret: String::new(),
                host: String::new(),
                api_token: String::new(),
            },
            jira: None,
            ldap: None,
        }
    }
}

impl JiraConfig {
    pub fn base_url(&self) -> String {
        if self.host.starts_with("http") {
            self.host.clone()
        } else {
            format!("https://{}", self.host)
        }
    }

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

    pub fn fix_versions(&self) -> String {
        if let Some(ref field) = self.fix_versions_field {
            field.clone()
        } else {
            "fixVersions".into()
        }
    }
}

pub fn parse(config_file: &str) -> Result<Config, String> {
    let mut config_file_open = match fs::File::open(config_file) {
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
    let config = parse_string(config_contents)?;

    let the_config = Config::new_with_model(config, users::UserConfig::new(), repos::RepoConfig::new());
    the_config.reload_users_repos()?;

    Ok(the_config)
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
