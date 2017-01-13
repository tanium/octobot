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
        let mut repo_map = RepoMap::new();
        repo_map.insert("some-user/the-repo".to_string(),
                        RepoInfo { channel: "the-repo-reviews".to_string() });

        let mut host_map = RepoHostMap::new();
        host_map.insert("git.company.com".to_string(), repo_map);

        let repos = RepoConfig { repos: host_map };

        let repo = github::Repo {
            html_url: "http://git.company.com/some-user/the-repo".to_string(),
            full_name: "some-user/the-repo".to_string(),
            owner: github::User { login: "someone-else".to_string() },
        };
        assert_eq!("the-repo-reviews", repos.lookup_channel(&repo).unwrap());
    }

    #[test]
    fn lookup_channel_by_repo_owner() {
        let mut repo_map = RepoMap::new();
        repo_map.insert("some-user".to_string(),
                        RepoInfo { channel: "the-repo-reviews".to_string() });

        let mut host_map = RepoHostMap::new();
        host_map.insert("git.company.com".to_string(), repo_map);

        let repos = RepoConfig { repos: host_map };

        let repo = github::Repo {
            html_url: "http://git.company.com/some-user/the-repo".to_string(),
            full_name: "some-user/some-other-repo".to_string(),
            owner: github::User { login: "some-user".to_string() },
        };
        assert_eq!("the-repo-reviews", repos.lookup_channel(&repo).unwrap());
    }

    #[test]
    fn lookup_channel_none() {
        let mut repo_map = RepoMap::new();
        repo_map.insert("some-user".to_string(),
                        RepoInfo { channel: "the-repo-reviews".to_string() });

        let mut host_map = RepoHostMap::new();
        host_map.insert("git.company.com".to_string(), repo_map);

        let repos = RepoConfig { repos: host_map };

        // fail by channel/repo
        {
            let repo = github::Repo {
                html_url: "http://git.company.com/some-user/the-repo".to_string(),
                full_name: "someone-else/some-other-repo".to_string(),
                owner: github::User { login: "someone-else".to_string() },
            };
            assert!(repos.lookup_channel(&repo).is_none());
        }

        // fail by git host
        {
            let repo = github::Repo {
                html_url: "http://git.other-company.com/some-user/the-repo".to_string(),
                full_name: "some-user/some-other-repo".to_string(),
                owner: github::User { login: "some-user".to_string() },
            };
            assert!(repos.lookup_channel(&repo).is_none());
        }
    }

}
