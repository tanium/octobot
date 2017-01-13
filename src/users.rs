use super::std;

use std::collections::HashMap;
use std::io::Read;
use rustc_serialize::json;
use url::Url;

use github;

#[derive(RustcDecodable, RustcEncodable, Clone)]
pub struct UserInfo {
    pub slack: String,
}

// maps git user name to user config
pub type UserMap = HashMap<String, UserInfo>;

// maps github host to user map
pub type UserHostMap = HashMap<String, UserMap>;

#[derive(Clone)]
pub struct UserConfig {
    users: UserHostMap,
}

pub fn load_config(file: &str) -> std::io::Result<UserConfig> {
    let mut f = try!(std::fs::File::open(file));
    let mut contents = String::new();
    try!(f.read_to_string(&mut contents));

    let users: UserHostMap = json::decode(&contents)
        .expect("Invalid JSON in users configuration file");

    Ok(UserConfig { users: users })
}

impl UserConfig {
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
            .map(|a| self.slack_user_name(a.login.as_str(), repo))
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
                    .and_then(|m| m.get(login))
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
    use super::super::github;

    #[test]
    fn test_slack_user_name_defaults() {
        let host_map = UserHostMap::new();
        let users = UserConfig { users: host_map };

        let repo = github::Repo {
            html_url: "http://git.company.com/some-user/the-repo".to_string(),
            full_name: "".to_string(),
            owner: github::User { login: "".to_string() },
        };

        assert_eq!("joe", users.slack_user_name("joe", &repo));
        assert_eq!("@joe", users.slack_user_ref("joe", &repo));
        assert_eq!("joe.smith", users.slack_user_name("joe-smith", &repo));
        assert_eq!("@joe.smith", users.slack_user_ref("joe-smith", &repo));
    }

    #[test]
    fn test_slack_user_name() {
        let mut user_map = UserMap::new();
        user_map.insert("some-git-user".to_string(),
                        UserInfo { slack: "the-slacker".to_string() });

        let mut host_map = UserHostMap::new();
        host_map.insert("git.company.com".to_string(), user_map);

        let users = UserConfig { users: host_map };

        let repo = github::Repo {
            html_url: "http://git.company.com/some-user/the-repo".to_string(),
            full_name: "some-user/the-repo".to_string(),
            owner: github::User { login: "someone-else".to_string() },
        };
        assert_eq!("the-slacker", users.slack_user_name("some-git-user", &repo));
        assert_eq!("@the-slacker", users.slack_user_ref("some-git-user", &repo));
        assert_eq!("some.other.user",
                   users.slack_user_name("some.other.user", &repo));
        assert_eq!("@some.other.user",
                   users.slack_user_ref("some.other.user", &repo));
    }

    #[test]
    fn test_slack_user_name_wrong_repo() {
        let mut user_map = UserMap::new();
        user_map.insert("some-user".to_string(),
                        UserInfo { slack: "the-repo-reviews".to_string() });

        let mut host_map = UserHostMap::new();
        host_map.insert("git.company.com".to_string(), user_map);

        let users = UserConfig { users: host_map };

        // fail by git host
        {
            let repo = github::Repo {
                html_url: "http://git.other-company.com/some-user/the-repo".to_string(),
                full_name: "some-user/some-other-repo".to_string(),
                owner: github::User { login: "some-user".to_string() },
            };
            assert_eq!("some.user", users.slack_user_name("some.user", &repo));
            assert_eq!("@some.user", users.slack_user_ref("some.user", &repo));
        }
    }

    #[test]
    fn test_mention() {
        assert_eq!("@me", mention("me"));
    }

}
