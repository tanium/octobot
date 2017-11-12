use serde_json;
use std;
use std::collections::HashMap;
use std::io::Read;
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
    // list of branches this jira/version config is for
    pub branches: Option<Vec<String>>,
    // A list of jira projects to be respected in processing.
    pub jira_projects: Option<Vec<String>>,
    pub jira_versions_enabled: Option<bool>,
    pub version_script: Option<String>,
    // Used for backporting. Defaults to "release/"
    pub release_branch_prefix: Option<String>,
}

// maps github host to a list of repos
pub type RepoHostMap = HashMap<String, Vec<RepoInfo>>;

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct RepoConfig {
    repos: RepoHostMap,
}

pub fn load_config(file: &str) -> std::io::Result<RepoConfig> {
    let mut f = std::fs::File::open(file)?;
    let mut contents = String::new();
    f.read_to_string(&mut contents)?;

    let repos: RepoHostMap = serde_json::from_str(&contents).expect("Invalid JSON in repos configuration file");

    Ok(RepoConfig { repos: repos })
}

impl RepoInfo {
    pub fn new(repo: &str, channel: &str) -> RepoInfo {
        RepoInfo {
            repo: repo.into(),
            branches: None,
            channel: channel.into(),
            force_push_notify: None,
            force_push_reapply_statuses: None,
            jira_projects: None,
            jira_versions_enabled: None,
            version_script: None,
            release_branch_prefix: None,
        }
    }

    pub fn with_branches(self, value: Vec<String>) -> RepoInfo {
        let mut info = self;
        info.branches = Some(value);
        info
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

    pub fn with_release_branch_prefix(self, value: Option<String>) -> RepoInfo {
        let mut info = self;
        info.release_branch_prefix = value;
        info
    }
}

impl RepoConfig {
    pub fn new() -> RepoConfig {
        RepoConfig { repos: RepoHostMap::new() }
    }

    pub fn from_github_host(host: &str) -> RepoConfig {
        let mut config = RepoConfig { repos: RepoHostMap::new() };
        config.repos.insert(host.into(), vec![]);
        config
    }

    pub fn insert(&mut self, host: &str, repo: &str, channel: &str) {
        self.insert_info(host, RepoInfo::new(repo, channel));
    }

    pub fn insert_info(&mut self, host: &str, info: RepoInfo) {
        self.repos.entry(host.to_string()).or_insert(vec![]).push(info);
    }

    pub fn lookup_channel(&self, repo: &github::Repo) -> Option<String> {
        match self.lookup_info(repo, None) {
            Some(info) => Some(info.channel.clone()),
            None => None,
        }
    }

    // never notify for unconfigured repos/orgs;
    // defaults to true for configured repos/orgs w/ no value set
    pub fn notify_force_push(&self, repo: &github::Repo) -> bool {
        match self.lookup_info(repo, None) {
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
        match self.lookup_info(repo, None) {
            None => vec![],
            Some(ref info) => {
                match info.force_push_reapply_statuses {
                    Some(ref value) => value.clone(),
                    None => vec![],
                }
            }
        }
    }

    pub fn jira_projects(&self, repo: &github::Repo, branch: &str) -> Vec<String> {
        match self.lookup_info(repo, Some(branch)) {
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
    pub fn jira_versions_enabled(&self, repo: &github::Repo, branch: &str) -> bool {
        match self.lookup_info(repo, Some(branch)) {
            None => false,
            Some(ref info) => {
                match info.jira_versions_enabled {
                    Some(value) => value,
                    None => false,
                }
            }
        }
    }

    pub fn version_script(&self, repo: &github::Repo, branch: &str) -> Option<String> {
        match self.lookup_info(repo, Some(branch)) {
            None => None,
            Some(ref info) => {
                match info.version_script {
                    Some(ref value) if value.len() > 0 => Some(value.clone()),
                    _ => None,
                }
            }
        }
    }

    pub fn release_branch_prefix(&self, repo: &github::Repo, branch: &str) -> String {
        let default = "release/".to_string();
        match self.lookup_info(repo, Some(branch)) {
            None => default,
            Some(ref info) => {
                match info.release_branch_prefix {
                    Some(ref prefix) => prefix.clone(),
                    None => default,
                }
            }
        }
    }

    fn lookup_info(&self, repo: &github::Repo, maybe_branch: Option<&str>) -> Option<&RepoInfo> {
        if let Ok(url) = Url::parse(&repo.html_url) {
            return url.host_str().and_then(|host| self.repos.get(host)).and_then(|repos| {
                // try to match by branch
                if let Some(branch) = maybe_branch {
                    for r in repos {
                        if r.repo == repo.full_name &&
                            r.branches.clone().map_or(false, |b| b.contains(&branch.to_string()))
                        {
                            return Some(r);
                        }
                    }
                }
                // try to match by org/repo
                for r in repos {
                    if r.repo == repo.full_name {
                        return Some(r);
                    }
                }
                // try to match by org
                for r in repos {
                    if r.repo == repo.owner.login() {
                        return Some(r);
                    }
                }

                None
            });
        }

        None
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
            let repo = github::Repo::parse("http://git.company.com/someone-else/some-other-repo").unwrap();
            assert!(repos.lookup_channel(&repo).is_none());
        }

        // fail by git host
        {
            let repo = github::Repo::parse("http://git.other-company.com/some-user/the-repo").unwrap();
            assert!(repos.lookup_channel(&repo).is_none());
        }
    }

    #[test]
    fn test_notify_force_push() {
        let mut repos = RepoConfig::new();
        repos.insert_info(
            "git.company.com",
            RepoInfo::new("some-user/noisy-repo-by-default", "reviews"),
        );
        repos.insert_info(
            "git.company.com",
            RepoInfo::new("some-user/noisy-repo-on-purpose", "reviews").with_force_push(Some(true)),
        );
        repos.insert_info(
            "git.company.com",
            RepoInfo::new("some-user/quiet-repo", "reviews").with_force_push(Some(false)),
        );
        {
            let repo = github::Repo::parse("http://git.company.com/someone-else/some-other-repo").unwrap();
            assert_eq!(false, repos.notify_force_push(&repo));
        }

        {
            let repo = github::Repo::parse(
                "http://git.company.\
                                            com/some-user/noisy-repo-by-default",
            ).unwrap();
            assert_eq!(true, repos.notify_force_push(&repo));
        }

        {
            let repo = github::Repo::parse(
                "http://git.company.\
                                            com/some-user/noisy-repo-on-purpose",
            ).unwrap();
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
        repos.insert_info("git.company.com", RepoInfo::new("some-user/no-config", "reviews"));
        repos.insert_info(
            "git.company.com",
            RepoInfo::new("some-user/with-config", "reviews").with_jira(vec!["a".into(), "b".into()]),
        );

        {
            let repo = github::Repo::parse("http://git.company.com/someone-else/some-other-repo").unwrap();
            assert_eq!(Vec::<String>::new(), repos.jira_projects(&repo, "any"));
        }

        {
            let repo = github::Repo::parse("http://git.company.com/some-user/no-config").unwrap();
            assert_eq!(Vec::<String>::new(), repos.jira_projects(&repo, "any"));
        }

        {
            let repo = github::Repo::parse("http://git.company.com/some-user/with-config").unwrap();
            assert_eq!(vec!["a", "b"], repos.jira_projects(&repo, "any"));
        }

    }

    #[test]
    fn test_jira_by_branch() {
        let mut repos = RepoConfig::new();
        repos.insert("git.company.com", "some-user", "SOME_OTHER_CHANNEL");

        repos.insert_info(
            "git.company.com",
            RepoInfo::new("some-user/the-repo", "the-repo-reviews").with_jira(vec!["SOME".into()]),
        );

        repos.insert_info(
            "git.company.com",
            RepoInfo::new("some-user/the-repo", "the-repo-reviews")
                .with_branches(vec!["the-branch".to_string()])
                .with_jira(vec!["THE-BRANCH".into()]),
        );

        let repo = github::Repo::parse("http://git.company.com/some-user/the-repo").unwrap();

        assert_eq!(vec!["THE-BRANCH"], repos.jira_projects(&repo, "the-branch"));
        assert_eq!(vec!["SOME"], repos.jira_projects(&repo, "any-other-branch"));
    }
}
