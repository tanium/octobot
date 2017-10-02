use std;
use std::collections::HashMap;
use std::io::Read;
use serde_json;
use url::Url;

use github;

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct RepoInfo {
    // github org or full repo name. i.e. "some-org" or "some-org/octobot"
    pub repo: String,
    // slack channel to send all messages to
    pub channel: String,
    pub force_push_notify: Option<bool>,
    // white-listed statuses to reapply on force-push w/ identical diff
    pub force_push_reapply_statuses: Option<Vec<String>>,
    // A list of jira projects to be respected in processing.
    pub jira_projects: Option<Vec<String>>,
    pub jira_versions_enabled: Option<bool>,
    pub version_script: Option<String>,
}

// maps github host to a list of repos
pub type RepoHostMap = HashMap<String, Vec<RepoInfo>>;

#[derive(Deserialize, Serialize, Clone, Debug)]
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

impl RepoInfo {
    pub fn new(repo: &str, channel: &str) -> RepoInfo {
        RepoInfo {
            repo: repo.into(),
            channel: channel.into(),
            force_push_notify: None,
            force_push_reapply_statuses: None,
            jira_projects: None,
            jira_versions_enabled: None,
            version_script: None,
        }
    }

    pub fn with_force_push(self, value: Option<bool>) -> RepoInfo {
        let mut info = self;
        info.force_push_notify = value;
        info
    }

    pub fn with_jira(self, value: Vec<String>) -> RepoInfo {
        let mut info = self;
        info.jira_projects = Some(value);
        info
    }

    pub fn with_version_script(self, value: Option<String>) -> RepoInfo {
        let mut info = self;
        info.version_script = value;
        info
    }
}

impl RepoConfig {
    pub fn new() -> RepoConfig {
        RepoConfig { repos: RepoHostMap::new() }
    }

    pub fn insert(&mut self, host: &str, repo: &str, channel: &str) {
        self.insert_info(host, RepoInfo::new(repo, channel));
    }

    pub fn insert_info(&mut self, host: &str, info: RepoInfo) {
        self.repos
            .entry(host.to_string())
            .or_insert(vec![])
            .push(info);
    }

    pub fn lookup_channel(&self, repo: &github::Repo) -> Option<String> {
        match self.lookup_info(repo) {
            Some(info) => Some(info.channel.clone()),
            None => None,
        }
    }

    // never notify for unconfigured repos/orgs;
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

    pub fn force_push_reapply_statuses(&self, repo: &github::Repo) -> Vec<String> {
        match self.lookup_info(repo) {
            None => vec![],
            Some(ref info) => {
                match info.force_push_reapply_statuses {
                    Some(ref value) => value.clone(),
                    None => vec![],
                }
            }
        }
    }

    pub fn jira_projects(&self, repo: &github::Repo) -> Vec<String>{
        match self.lookup_info(repo) {
            None => vec![],
            Some(ref info) => {
                match info.jira_projects {
                    Some(ref value) => value.clone(),
                    None => vec![],
                }
            }
        }
    }

    // never enable on unconfigured repos/orgs;
    // defaults to true for configured repos/orgs w/ no value set
    pub fn jira_versions_enabled(&self, repo: &github::Repo) -> bool {
        match self.lookup_info(repo) {
            None => false,
            Some(ref info) => {
                match info.jira_versions_enabled {
                    Some(value) => value,
                    None => true,
                }
            }
        }
    }

    pub fn version_script(&self, repo: &github::Repo) -> Option<String> {
        match self.lookup_info(repo) {
            None => None,
            Some(ref info) => {
                match info.version_script {
                    Some(ref value) if value.len() > 0 => Some(value.clone()),
                    _ => None
                }
            }
        }
    }

    fn lookup_info(&self, repo: &github::Repo) -> Option<&RepoInfo> {
        match Url::parse(&repo.html_url) {
            Ok(u) => {
                u.host_str()
                    .and_then(|h| self.repos.get(h))
                    .and_then(|repos| {
                        // try to find most-specific first, then look for org-level match
                        match repos.iter().find(|r| r.repo == repo.full_name) {
                            Some(r) => Some(r),
                            None => repos.iter().find(|r| r.repo == repo.owner.login())
                        }
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
        // insert org-level one first in the list to make sure most specific matches first
        repos.insert("git.company.com", "some-user", "SOME_OTHER_CHANNEL");
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
                          RepoInfo::new("some-user/noisy-repo-by-default", "reviews"));
        repos.insert_info("git.company.com",
                          RepoInfo::new("some-user/noisy-repo-on-purpose", "reviews").with_force_push(Some(true)));
        repos.insert_info("git.company.com",
                          RepoInfo::new("some-user/quiet-repo", "reviews").with_force_push(Some(false)));
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

    #[test]
    fn test_jira_enabled() {
        let mut repos = RepoConfig::new();
        repos.insert_info("git.company.com",
                          RepoInfo::new("some-user/no-config", "reviews"));
        repos.insert_info("git.company.com",
                          RepoInfo::new("some-user/with-config", "reviews").with_jira(vec!["a".into(), "b".into()]));

        {
            let repo = github::Repo::parse("http://git.company.com/someone-else/some-other-repo")
                .unwrap();
            assert_eq!(Vec::<String>::new(), repos.jira_projects(&repo));
        }

        {
            let repo = github::Repo::parse("http://git.company.com/some-user/no-config")
                .unwrap();
            assert_eq!(Vec::<String>::new(), repos.jira_projects(&repo));
        }

        {
            let repo = github::Repo::parse("http://git.company.com/some-user/with-config")
                .unwrap();
            assert_eq!(vec!["a", "b"], repos.jira_projects(&repo));
        }

    }
}
