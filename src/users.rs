use std;
use std::collections::HashMap;
use std::io::Read;
use serde_json;
use url::Url;

use github;

#[derive(Deserialize, Serialize, Clone)]
pub struct UserInfo {
    pub github: String,
    pub slack: String,
}

// maps github host to list of users
pub type UserHostMap = HashMap<String, Vec<UserInfo>>;

#[derive(Deserialize, Serialize, Clone)]
pub struct UserConfig {
    users: UserHostMap,
}

pub fn load_config(file: &str) -> std::io::Result<UserConfig> {
    let mut f = try!(std::fs::File::open(file));
    let mut contents = String::new();
    try!(f.read_to_string(&mut contents));

    let users: UserHostMap = serde_json::from_str(&contents)
        .expect("Invalid JSON in users configuration file");

    Ok(UserConfig { users: users })
}

impl UserConfig {
    pub fn new() -> UserConfig {
        UserConfig { users: UserHostMap::new() }
    }

    pub fn insert(&mut self, host: &str, git_user: &str, slack_user: &str) {
        self.users
            .entry(host.to_string())
            .or_insert(vec![])
            .push(UserInfo {
                github: git_user.to_string(),
                slack: slack_user.to_string(),
            });
    }

    // our slack convention is to use '.' but github replaces dots with dashes.
    pub fn slack_user_name<S: Into<String>>(&self, login: S, repo: &github::Repo) -> String {
        let login = login.into();
        match self.lookup_name(login.as_str(), repo) {
            Some(name) => name,
            None => login.as_str().replace('-', "."),
        }
    }

    pub fn slack_user_ref<S: Into<String>>(&self, login: S, repo: &github::Repo) -> String {
        mention(self.slack_user_name(login.into(), repo))
    }

    pub fn slack_user_names(&self, users: &Vec<github::User>, repo: &github::Repo) -> Vec<String> {
        users.iter()
            .map(|a| self.slack_user_name(a.login(), repo))
            .collect()
    }

    fn lookup_name(&self, login: &str, repo: &github::Repo) -> Option<String> {
        match self.lookup_info(login, repo) {
            Some(info) => Some(info.slack.clone()),
            None => None,
        }
    }

    fn lookup_info(&self, login: &str, repo: &github::Repo) -> Option<&UserInfo> {
        match Url::parse(&repo.html_url) {
            Ok(u) => {
                u.host_str()
                    .and_then(|h| self.users.get(h))
                    .and_then(|users| users.iter().find(|u| u.github == login))
            }
            Err(_) => None,
        }
    }
}

pub fn mention<S: Into<String>>(username: S) -> String {
    format!("@{}", username.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use github;

    #[test]
    fn test_slack_user_name_defaults() {
        let users = UserConfig::new();

        let repo = github::Repo::new();

        assert_eq!("joe", users.slack_user_name("joe", &repo));
        assert_eq!("@joe", users.slack_user_ref("joe", &repo));
        assert_eq!("joe.smith", users.slack_user_name("joe-smith", &repo));
        assert_eq!("@joe.smith", users.slack_user_ref("joe-smith", &repo));
    }

    #[test]
    fn test_slack_user_name() {
        let mut users = UserConfig::new();
        users.insert("git.company.com", "some-git-user", "the-slacker");

        let repo = github::Repo::parse("http://git.company.com/some-user/the-repo").unwrap();
        assert_eq!("the-slacker", users.slack_user_name("some-git-user", &repo));
        assert_eq!("@the-slacker", users.slack_user_ref("some-git-user", &repo));
        assert_eq!("some.other.user",
                   users.slack_user_name("some.other.user", &repo));
        assert_eq!("@some.other.user",
                   users.slack_user_ref("some.other.user", &repo));
    }

    #[test]
    fn test_slack_user_name_wrong_repo() {
        let mut users = UserConfig::new();
        users.insert("git.company.com", "some-user", "the-slacker");

        // fail by git host
        {
            let repo = github::Repo::parse("http://git.other-company.\
                                            com/some-user/some-other-repo")
                .unwrap();
            assert_eq!("some.user", users.slack_user_name("some.user", &repo));
            assert_eq!("@some.user", users.slack_user_ref("some.user", &repo));
        }
    }

    #[test]
    fn test_mention() {
        assert_eq!("@me", mention("me"));
    }

}
