use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

use anyhow::anyhow;
use serde_derive::{Deserialize, Serialize};
use toml;

use crate::config_db::ConfigDatabase;
use crate::errors::*;
use crate::repos;
use crate::users;

pub struct Config {
    pub main: MainConfig,
    pub admin: Option<AdminConfig>,
    pub metrics: Option<MetricsConfig>,
    pub github: GithubConfig,
    pub slack: SlackConfig,
    pub jira: Option<JiraConfig>,
    pub ldap: Option<LdapConfig>,

    pub users: RwLock<users::UserConfig>,
    pub repos: RwLock<repos::RepoConfig>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ConfigModel {
    pub main: MainConfig,
    pub admin: Option<AdminConfig>,
    pub metrics: Option<MetricsConfig>,
    pub github: GithubConfig,
    pub slack: SlackConfig,
    pub jira: Option<JiraConfig>,
    pub ldap: Option<LdapConfig>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MainConfig {
    pub listen_addr: Option<String>,
    pub clone_root_dir: String,
    pub num_http_threads: Option<usize>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AdminConfig {
    pub name: String,
    pub salt: String,
    pub pass_hash: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MetricsConfig {
    pub salt: String,
    pub pass_hash: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SlackConfig {
    pub bot_token: String,
    #[serde(default)]
    pub ignored_users: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct GithubConfig {
    pub webhook_secret: String,
    pub host: String,
    pub api_token: Option<String>,
    pub app_id: Option<u32>,
    pub app_key_file: Option<String>,
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
    // optional name of role to restrict octobot comment visibility. (e.g. "Developers")
    pub restrict_comment_visibility_to_role: Option<String>,
    // optional suffix to add to the username for the login dialog (e.g. "@company.com")
    pub login_suffix: Option<String>,
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
    // attributes to match logins against (e.g. ["samAccountName", "userPrincipalName"] for AD, ["uid, "mail"] for LDAP)
    pub userid_attributes: Vec<String>,
    // Additional LDAP search filter for user types and group membership
    // e.g. (&(objectCategory=Person)(memberOf=cn=octobot-admins,ou=users,dc=company,dc=com))
    pub search_filter: Option<String>,
}

impl Config {
    // TODO: weird that `new` is used only by tests and the actual `new` is below...
    pub fn new(db: ConfigDatabase) -> Config {
        Config::new_with_model(ConfigModel::new(), db)
    }

    fn new_with_model(config: ConfigModel, db: ConfigDatabase) -> Config {
        Config {
            main: config.main,
            admin: config.admin,
            metrics: config.metrics,
            github: config.github,
            slack: config.slack,
            jira: config.jira,
            ldap: config.ldap,
            users: RwLock::new(users::UserConfig::new(db.clone())),
            repos: RwLock::new(repos::RepoConfig::new(db)),
        }
    }

    pub fn save(&self, config_file: &str) -> Result<()> {
        let model = ConfigModel {
            main: self.main.clone(),
            admin: self.admin.clone(),
            metrics: self.metrics.clone(),
            github: self.github.clone(),
            slack: self.slack.clone(),
            jira: self.jira.clone(),
            ldap: self.ldap.clone(),
        };

        let serialized =
            toml::to_string(&model).map_err(|e| anyhow!("Error serializing config: {}", e))?;

        let tmp_file = config_file.to_string() + ".tmp";
        let bak_file = config_file.to_string() + ".bak";

        let mut file = fs::File::create(&tmp_file)?;

        file.write_all(serialized.as_bytes())?;
        fs::rename(config_file, &bak_file)?;
        fs::rename(&tmp_file, config_file)?;
        fs::remove_file(&bak_file)?;

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
                listen_addr: None,
                clone_root_dir: String::new(),
                num_http_threads: None,
            },
            admin: None,
            metrics: None,
            github: GithubConfig {
                webhook_secret: String::new(),
                host: String::new(),
                api_token: None,
                app_id: None,
                app_key_file: None,
            },
            slack: SlackConfig {
                bot_token: String::new(),
                ignored_users: vec![],
            },
            jira: None,
            ldap: None,
        }
    }
}

impl GithubConfig {
    pub fn app_key(&self) -> Result<Vec<u8>> {
        let key_file = &self
            .app_key_file
            .as_ref()
            .expect("expected an app_key_file");

        let mut file_open = fs::File::open(key_file)?;

        let mut contents = vec![];
        file_open.read_to_end(&mut contents)?;
        Ok(contents)
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

pub fn new(config_file: PathBuf) -> Result<Config> {
    let db_file_name = "db.sqlite3";
    match config_file.file_name() {
        Some(name) => {
            if name == db_file_name {
                return Err(anyhow!("Must provide toml config file"));
            }
        }
        None => return Err(anyhow!("Provided config file has no file name")),
    };

    let mut db_file = config_file.clone();
    db_file.set_file_name(db_file_name);
    let db = ConfigDatabase::new(&db_file.to_string_lossy())?;

    let mut config_file_open = fs::File::open(&config_file)?;
    let mut config_contents = String::new();
    config_file_open.read_to_string(&mut config_contents)?;
    let config_model = parse_string(&config_contents)?;

    Ok(Config::new_with_model(config_model, db))
}

fn parse_string(config_contents: &str) -> Result<ConfigModel> {
    toml::from_str::<ConfigModel>(config_contents)
        .map_err(|e| anyhow!("Error parsing config: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse() {
        let config_str = r#"
[main]
clone_root_dir = "./repos"

[slack]
bot_token = "foo"

[github]
webhook_secret = "abcd"
host = "git.company.com"
app_id = 2
app_key_file = "some-file.key"
"#;
        let config = parse_string(config_str).unwrap();
        assert_eq!("foo", config.slack.bot_token);
    }
}
