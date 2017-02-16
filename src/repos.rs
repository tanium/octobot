use std;
use std::collections::HashMap;
use std::io::Read;
use serde_json;
use url::Url;

use github;

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct RepoInfo {
    pub channel: String,
    pub force_push_notify: Option<bool>,
}

// maps repo name to repo config
pub type RepoMap = HashMap<String, RepoInfo>;

// maps github host to repos map
pub type RepoHostMap = HashMap<String, RepoMap>;

#[derive(Clone, Debug)]
pub struct RepoConfig {
    repos: RepoHostMap,
}

pub fn load_config(file: &str) -> std::io::Result<RepoConfig> {
    let mut f = try!(std::fs::File::open(file));
    let mut contents = String::new();
    try!(f.read_to_string(&mut contents));

    let repos: RepoHostMap = serde_json::from_str(&contents)
        .expect("Invalid JSON in repos configuration file");

    Ok(RepoConfig { repos: repos })
}

impl RepoConfig {
    pub fn new() -> RepoConfig {
        RepoConfig { repos: RepoHostMap::new() }
    }

    pub fn insert(&mut self, host: &str, repo_name: &str, channel: &str) {
        self.insert_info(host,
                         repo_name,
                         RepoInfo {
                             channel: channel.to_string(),
                             force_push_notify: None,
                         });
    }

    pub fn insert_info(&mut self, host: &str, repo_name: &str, info: RepoInfo) {
        self.repos
            .entry(host.to_string())
            .or_insert(RepoMap::new())
            .insert(repo_name.to_string(), info);
    }

    pub fn lookup_channel(&self, repo: &github::Repo) -> Option<String> {
        match self.lookup_info(repo) {
            Some(info) => Some(info.channel.clone()),
            None => None,
        }
    }

    // always force for unconfigured repos/orgs;
    // defaults to true for configured repos/orgs w/ no value set
    pub fn notify_force_push(&self, repo: &github::Repo) -> bool {
        match self.lookup_info(repo) {
            None => false,
            Some(ref info) => {
                match info.force_push_notify {
                    Some(value) => value,
                    None => true,
                }
            }
        }
    }

    fn lookup_info(&self, repo: &github::Repo) -> Option<&RepoInfo> {
        match Url::parse(&repo.html_url) {
            Ok(u) => {
                u.host_str()
                    .and_then(|h| self.repos.get(h))
                    .and_then(|m| {
                        m.get(&repo.full_name)
                            .or(m.get(repo.owner.login()))
                    })
            }
            Err(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use github;

    #[test]
    fn lookup_channel_by_repo_full_name() {
        let mut repos = RepoConfig::new();
        repos.insert("git.company.com", "some-user/the-repo", "the-repo-reviews");

        let repo = github::Repo::parse("http://git.company.com/some-user/the-repo").unwrap();
        assert_eq!("the-repo-reviews", repos.lookup_channel(&repo).unwrap());
    }

    #[test]
    fn lookup_channel_by_repo_owner() {
        let mut repos = RepoConfig::new();
        repos.insert("git.company.com", "some-user", "the-repo-reviews");

        let repo = github::Repo::parse("http://git.company.com/some-user/some-other-repo").unwrap();
        assert_eq!("the-repo-reviews", repos.lookup_channel(&repo).unwrap());
    }

    #[test]
    fn lookup_channel_none() {
        let mut repos = RepoConfig::new();
        repos.insert("git.company.com", "some-user", "the-repo-reviews");

        // fail by channel/repo
        {
            let repo = github::Repo::parse("http://git.company.com/someone-else/some-other-repo")
                .unwrap();
            assert!(repos.lookup_channel(&repo).is_none());
        }

        // fail by git host
        {
            let repo = github::Repo::parse("http://git.other-company.com/some-user/the-repo")
                .unwrap();
            assert!(repos.lookup_channel(&repo).is_none());
        }
    }

    #[test]
    fn test_notify_force_push() {
        let mut repos = RepoConfig::new();
        repos.insert_info("git.company.com",
                          "some-user/noisy-repo-by-default",
                          RepoInfo {
                              channel: "reviews".to_string(),
                              force_push_notify: None,
                          });
        repos.insert_info("git.company.com",
                          "some-user/noisy-repo-on-purpose",
                          RepoInfo {
                              channel: "reviews".to_string(),
                              force_push_notify: Some(true),
                          });
        repos.insert_info("git.company.com",
                          "some-user/quiet-repo",
                          RepoInfo {
                              channel: "reviews".to_string(),
                              force_push_notify: Some(false),
                          });

        {
            let repo = github::Repo::parse("http://git.company.com/someone-else/some-other-repo")
                .unwrap();
            assert_eq!(false, repos.notify_force_push(&repo));
        }

        {
            let repo = github::Repo::parse("http://git.company.\
                                            com/some-user/noisy-repo-by-default")
                .unwrap();
            assert_eq!(true, repos.notify_force_push(&repo));
        }

        {
            let repo = github::Repo::parse("http://git.company.\
                                            com/some-user/noisy-repo-on-purpose")
                .unwrap();
            assert_eq!(true, repos.notify_force_push(&repo));
        }

        {
            let repo = github::Repo::parse("http://git.company.com/some-user/quiet-repo").unwrap();
            assert_eq!(false, repos.notify_force_push(&repo));
        }

    }
}
