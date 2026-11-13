use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use tokio::time::Instant;

use octobot_lib::github;

// Who has submitted a review of each PR. A submitted review cannot be deleted, so
// this only grows: reviewers that webhooks report merge with those a lookup
// returns, and neither loses the other's however the two interleave.
pub struct ReviewersCache {
    entries: Mutex<HashMap<(String, u32), Entry>>,
    ttl: Duration,
}

struct Entry {
    reviewers: Vec<github::User>,
    // When a lookup last stored its reviewers. Until one has, webhooks may have
    // reported only some of them.
    looked_up: Option<Instant>,
    updated: Instant,
}

impl ReviewersCache {
    pub fn new(ttl: Duration) -> ReviewersCache {
        ReviewersCache {
            entries: Mutex::new(HashMap::new()),
            ttl,
        }
    }

    // Every reviewer, or none if no lookup has stored them within the TTL.
    pub fn get(&self, repo: &github::Repo, number: u32) -> Option<Vec<github::User>> {
        let entries = self.entries.lock().unwrap();
        let entry = entries.get(&key(repo, number))?;
        let looked_up = entry.looked_up?;
        if Instant::now() >= looked_up + self.ttl {
            return None;
        }
        Some(entry.reviewers.clone())
    }

    // Returns every reviewer known, including any a webhook reported while the
    // lookup ran.
    pub fn insert_looked_up(
        &self,
        repo: &github::Repo,
        number: u32,
        reviewers: Vec<github::User>,
    ) -> Vec<github::User> {
        let now = Instant::now();
        let mut entries = self.entries.lock().unwrap();
        let entry = entry(&mut entries, repo, number, now);
        merge(&mut entry.reviewers, reviewers);
        entry.looked_up = Some(now);
        entry.updated = now;
        entry.reviewers.clone()
    }

    pub fn add(&self, repo: &github::Repo, number: u32, reviewer: &github::User) {
        let now = Instant::now();
        let mut entries = self.entries.lock().unwrap();
        let entry = entry(&mut entries, repo, number, now);
        merge(&mut entry.reviewers, vec![reviewer.clone()]);
        entry.updated = now;
    }

    // Keeps an entry for a TTL after it last changed. That is far longer than a
    // lookup takes, so a reviewer a webhook reported is still here when a lookup
    // that started before it finishes.
    pub fn clean(&self) {
        let now = Instant::now();
        self.entries
            .lock()
            .unwrap()
            .retain(|_, entry| now < entry.updated + self.ttl);
    }

    pub fn entry_count(&self) -> usize {
        self.entries.lock().unwrap().len()
    }
}

fn key(repo: &github::Repo, number: u32) -> (String, u32) {
    (repo.full_name.clone(), number)
}

fn entry<'a>(
    entries: &'a mut HashMap<(String, u32), Entry>,
    repo: &github::Repo,
    number: u32,
    now: Instant,
) -> &'a mut Entry {
    entries.entry(key(repo, number)).or_insert_with(|| Entry {
        reviewers: vec![],
        looked_up: None,
        updated: now,
    })
}

fn merge(reviewers: &mut Vec<github::User>, more: Vec<github::User>) {
    for user in more {
        if !reviewers.iter().any(|r| r.login() == user.login()) {
            reviewers.push(user);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TTL: Duration = Duration::from_secs(3600);

    fn repo() -> github::Repo {
        github::Repo::parse("http://the-github-host/some-user/some-repo").unwrap()
    }

    fn users(logins: &[&str]) -> Vec<github::User> {
        logins.iter().map(|l| github::User::new(l)).collect()
    }

    #[tokio::test(start_paused = true)]
    async fn test_webhooks_alone_are_not_a_hit() {
        let cache = ReviewersCache::new(TTL);
        cache.add(&repo(), 32, &github::User::new("reviewer1"));
        assert_eq!(None, cache.get(&repo(), 32));
    }

    #[tokio::test(start_paused = true)]
    async fn test_merges_webhooks_and_lookups() {
        let cache = ReviewersCache::new(TTL);
        // A review lands while a lookup that predates it is still running.
        cache.add(&repo(), 32, &github::User::new("reviewer2"));
        let all = cache.insert_looked_up(&repo(), 32, users(&["reviewer1"]));
        assert_eq!(users(&["reviewer2", "reviewer1"]), all);

        cache.add(&repo(), 32, &github::User::new("reviewer3"));
        cache.add(&repo(), 32, &github::User::new("reviewer1"));
        assert_eq!(
            Some(users(&["reviewer2", "reviewer1", "reviewer3"])),
            cache.get(&repo(), 32)
        );
        assert_eq!(None, cache.get(&repo(), 33));
    }

    #[tokio::test(start_paused = true)]
    async fn test_expires_from_lookup_not_webhook() {
        let cache = ReviewersCache::new(TTL);
        cache.insert_looked_up(&repo(), 32, users(&["reviewer1"]));
        tokio::time::advance(TTL / 2).await;
        cache.add(&repo(), 32, &github::User::new("reviewer2"));
        tokio::time::advance(TTL / 2).await;
        assert_eq!(None, cache.get(&repo(), 32));

        // The reviewer a webhook reported survives into the next lookup.
        let all = cache.insert_looked_up(&repo(), 32, users(&["reviewer1"]));
        assert_eq!(users(&["reviewer1", "reviewer2"]), all);
    }

    #[tokio::test(start_paused = true)]
    async fn test_clean_keeps_recent_webhooks() {
        let cache = ReviewersCache::new(TTL);
        cache.insert_looked_up(&repo(), 32, users(&["reviewer1"]));
        cache.add(&repo(), 33, &github::User::new("reviewer1"));
        tokio::time::advance(TTL / 2).await;
        cache.add(&repo(), 32, &github::User::new("reviewer2"));
        tokio::time::advance(TTL / 2).await;

        cache.clean();
        assert_eq!(1, cache.entry_count());
        let entries = cache.entries.lock().unwrap();
        assert!(entries.contains_key(&key(&repo(), 32)));
        assert!(!entries.contains_key(&key(&repo(), 33)));
    }
}
