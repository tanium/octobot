use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use tokio::time::Instant;

use octobot_lib::github;

// Each PR's commits, keyed by its head and base, so a push or a base change makes
// a new key rather than changing an old one. GitHub moves a PR's base only when
// the PR changes, not when its base branch does, so busy base branches still hit.
// That same base may hide commits that land in the base branch through a merge
// commit and leave the PR's list; the TTL bounds how long those linger.
pub struct CommitsCache {
    entries: Mutex<HashMap<Key, (Vec<github::Commit>, Instant)>>,
    ttl: Duration,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Key {
    repo: String,
    number: u32,
    head: String,
    base: String,
}

impl Key {
    fn new(pull_request: &github::PullRequest) -> Key {
        Key {
            repo: pull_request.base.repo.full_name.clone(),
            number: pull_request.number,
            head: pull_request.head.sha.clone(),
            base: pull_request.base.sha.clone(),
        }
    }
}

impl CommitsCache {
    pub fn new(ttl: Duration) -> CommitsCache {
        CommitsCache {
            entries: Mutex::new(HashMap::new()),
            ttl,
        }
    }

    pub fn get(&self, pull_request: &github::PullRequest) -> Option<Vec<github::Commit>> {
        let entries = self.entries.lock().unwrap();
        let (commits, expiry) = entries.get(&Key::new(pull_request))?;
        if Instant::now() >= *expiry {
            return None;
        }
        Some(commits.clone())
    }

    // Stores the commits only if they end at the PR's head. A lookup that ran after
    // a push the PR doesn't show yet returns the newer commits, which must not be
    // filed under the older head.
    pub fn insert(&self, pull_request: &github::PullRequest, commits: Vec<github::Commit>) {
        if commits.last().map(|c| c.sha.as_str()) != Some(pull_request.head.sha.as_str()) {
            return;
        }
        let expiry = Instant::now() + self.ttl;
        self.entries
            .lock()
            .unwrap()
            .insert(Key::new(pull_request), (commits, expiry));
    }

    pub fn clean(&self) {
        let now = Instant::now();
        self.entries
            .lock()
            .unwrap()
            .retain(|_, (_, expiry)| now < *expiry);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TTL: Duration = Duration::from_secs(3600);

    fn commit(sha: &str) -> github::Commit {
        github::Commit {
            sha: sha.into(),
            html_url: String::new(),
            commit: github::CommitDetails {
                message: String::new(),
            },
            author: None,
        }
    }

    fn pr(number: u32, head: &str, base: &str) -> github::PullRequest {
        let mut pr = github::PullRequest::new();
        pr.number = number;
        pr.head.sha = head.into();
        pr.base.sha = base.into();
        pr.base.repo = github::Repo::parse("http://the-github-host/some-user/some-repo").unwrap();
        pr
    }

    #[tokio::test(start_paused = true)]
    async fn test_keyed_by_head_and_base() {
        let cache = CommitsCache::new(TTL);
        let commits = vec![commit("aaa"), commit("bbb")];
        cache.insert(&pr(32, "bbb", "base1"), commits.clone());

        assert_eq!(Some(commits), cache.get(&pr(32, "bbb", "base1")));
        assert_eq!(None, cache.get(&pr(32, "bbb", "base2")));
        assert_eq!(None, cache.get(&pr(32, "ccc", "base1")));
        assert_eq!(None, cache.get(&pr(33, "bbb", "base1")));
    }

    #[tokio::test(start_paused = true)]
    async fn test_skips_commits_past_the_head() {
        let cache = CommitsCache::new(TTL);
        cache.insert(&pr(32, "aaa", "base1"), vec![commit("aaa"), commit("bbb")]);
        assert_eq!(None, cache.get(&pr(32, "aaa", "base1")));

        cache.insert(&pr(32, "aaa", "base1"), vec![]);
        assert_eq!(None, cache.get(&pr(32, "aaa", "base1")));
    }

    #[tokio::test(start_paused = true)]
    async fn test_expires() {
        let cache = CommitsCache::new(TTL);
        cache.insert(&pr(32, "aaa", "base1"), vec![commit("aaa")]);
        tokio::time::advance(TTL - Duration::from_secs(1)).await;
        assert!(cache.get(&pr(32, "aaa", "base1")).is_some());

        tokio::time::advance(Duration::from_secs(1)).await;
        assert_eq!(None, cache.get(&pr(32, "aaa", "base1")));
    }

    #[tokio::test(start_paused = true)]
    async fn test_clean_drops_only_expired() {
        let cache = CommitsCache::new(TTL);
        cache.insert(&pr(32, "aaa", "base1"), vec![commit("aaa")]);
        tokio::time::advance(TTL / 2).await;
        cache.insert(&pr(33, "aaa", "base1"), vec![commit("aaa")]);
        tokio::time::advance(TTL / 2).await;

        cache.clean();
        let entries = cache.entries.lock().unwrap();
        assert!(!entries.contains_key(&Key::new(&pr(32, "aaa", "base1"))));
        assert!(entries.contains_key(&Key::new(&pr(33, "aaa", "base1"))));
    }
}
