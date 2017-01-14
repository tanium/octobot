use super::std;

use std::collections::HashMap;
use std::io::Read;
use rustc_serialize::json;
use url::Url;

use github;

#[derive(RustcDecodable, RustcEncodable, Clone)]
pub struct RepoInfo {
    pub channel: String,
}

// maps repo name to repo config
pub type RepoMap = HashMap<String, RepoInfo>;

// maps github host to repos map
pub type RepoHostMap = HashMap<String, RepoMap>;

#[derive(Clone)]
pub struct RepoConfig {
    repos: RepoHostMap,
}

pub fn load_config(file: &str) -> std::io::Result<RepoConfig> {
    let mut f = try!(std::fs::File::open(file));
    let mut contents = String::new();
    try!(f.read_to_string(&mut contents));

    let repos: RepoHostMap = json::decode(&contents)
        .expect("Invalid JSON in repos configuration file");

    Ok(RepoConfig { repos: repos })
}

impl RepoConfig {

    pub fn new() -> RepoConfig {
        RepoConfig {
            repos: RepoHostMap::new()
        }
    }

    pub fn insert(&mut self, host: &str, repo_name: &str, channel: &str) {
        self.repos
            .entry(host.to_string())
            .or_insert(RepoMap::new())
            .insert(repo_name.to_string(),
                    RepoInfo { channel: channel.to_string() });
    }

    pub fn lookup_channel(&self, repo: &github::Repo) -> Option<String> {
        match self.lookup_info(repo) {
            Some(info) => Some(info.channel.clone()),
            None => None,
        }
    }

    fn lookup_info(&self, repo: &github::Repo) -> Option<&RepoInfo> {
        match Url::parse(&repo.html_url) {
            Ok(u) => {
                u.host_str()
                    .and_then(|h| self.repos.get(h))
                    .and_then(|m| m.get(&repo.full_name).or(m.get(&repo.owner.login)))
            }
            Err(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::github;

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
            let repo = github::Repo::parse("http://git.company.com/someone-else/some-other-repo").unwrap();
            assert!(repos.lookup_channel(&repo).is_none());
        }

        // fail by git host
        {
            let repo = github::Repo::parse("http://git.other-company.com/some-user/the-repo").unwrap();
            assert!(repos.lookup_channel(&repo).is_none());
        }
    }

}
