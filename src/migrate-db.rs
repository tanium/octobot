extern crate octobot;
extern crate serde;
#[macro_use]
extern crate serde_derive;
extern crate serde_json;

use std::collections::HashMap;
use std::io::Read;

use serde::de::DeserializeOwned;

use octobot::db;
use octobot::errors::*;
use octobot::repos;
use octobot::users;

#[derive(Deserialize, Serialize, Clone)]
struct UserInfoJSON {
    pub github: String,
    pub slack: String,
}

type UserHostMap = HashMap<String, Vec<UserInfoJSON>>;

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct RepoInfoJSON {
    pub repo: String,
    pub channel: String,
    pub force_push_notify: Option<bool>,
    pub force_push_reapply_statuses: Option<Vec<String>>,
    pub branches: Option<Vec<String>>,
    pub jira_projects: Option<Vec<String>>,
    pub jira_versions_enabled: Option<bool>,
    pub version_script: Option<String>,
    pub release_branch_prefix: Option<String>,
}

pub type RepoHostMap = HashMap<String, Vec<RepoInfoJSON>>;


fn main() {
    if let Err(ref e) = run() {
        use std::io::Write;
        let stderr = &mut ::std::io::stderr();
        let errmsg = "Error writing to stderr";

        writeln!(stderr, "error: {}", e).expect(errmsg);

        for e in e.iter().skip(1) {
            writeln!(stderr, "caused by: {}", e).expect(errmsg);
        }

        ::std::process::exit(1);
    }
}

fn run() -> Result<()> {
    if std::env::args().len() < 4 {
        return Err("Usage: migrate-db <db-file> <users.json> <repos.json>".into());
    }

    let db_file = std::env::args().nth(1).unwrap();
    let users_json = std::env::args().nth(2).unwrap();
    let repos_json = std::env::args().nth(3).unwrap();

    let db = db::Database::new(&db_file)?;

    let mut users_db = users::UserConfig::new(db.clone());
    let users_map = load_config::<UserHostMap>(&users_json)?;

    let mut repos_db = repos::RepoConfig::new(db.clone());
    let repos_map = load_config::<RepoHostMap>(&repos_json)?;

    for (_, users) in &users_map {
        for user in users {
            if let Err(e) = users_db.insert(&user.github, &user.slack) {
                println!("Error adding user {}: {}", user.github, e);
            } else {
                println!("Added user: {}", user.github);
            }
        }
    }

    for (_, repos) in &repos_map {
        for info in repos {
            let repo = repos::RepoInfo {
                id: None,
                repo: info.repo.clone(),
                channel: info.channel.clone(),
                force_push_notify: info.force_push_notify.unwrap_or(false),
                force_push_reapply_statuses: info.force_push_reapply_statuses.clone().unwrap_or(vec![]),
                branches: info.branches.clone().unwrap_or(vec![]),
                jira_projects: info.jira_projects.clone().unwrap_or(vec![]),
                jira_versions_enabled: info.jira_versions_enabled.unwrap_or(false),
                version_script: info.version_script.clone().unwrap_or(String::new()),
                release_branch_prefix: info.release_branch_prefix.clone().unwrap_or(String::new()),
            };

            if let Err(e) = repos_db.insert_info(&repo) {
                println!("Error adding repo {}: {}", info.repo, e);
            } else {
                println!("Added repo: {}", info.repo);
            }
        }
    }

    Ok(())
}

fn load_config<T: DeserializeOwned>(file: &str) -> Result<T> {
    let mut f = std::fs::File::open(file)?;
    let mut contents = String::new();
    f.read_to_string(&mut contents)?;
    serde_json::from_str(&contents).map_err(|_| Error::from("Error parsing json"))
}
