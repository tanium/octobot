use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use tokio::time::Instant;

use octobot_lib::github;

// The people on each PR, for events that name a PR without carrying it (check
// runs and suites). Every PR a webhook or lookup carries updates it, so it needs
// no API calls of its own beyond a lookup on a miss.
pub struct PeopleCache {
    entries: Mutex<HashMap<(String, u32), Entry>>,
    ttl: Duration,
}

#[derive(Clone, Debug, PartialEq)]
pub struct People {
    pub author: github::User,
    pub assignees: Vec<github::User>,
    pub requested_reviewers: Vec<github::User>,
    pub requested_teams: Vec<github::Team>,
}

impl People {
    fn new(pull_request: &github::PullRequest) -> People {
        People {
            author: pull_request.user.clone(),
            assignees: pull_request.assignees.clone(),
            requested_reviewers: pull_request.requested_reviewers.clone().unwrap_or_default(),
            requested_teams: pull_request.requested_teams.clone().unwrap_or_default(),
        }
    }
}

struct Entry {
    // None after a tie, so a late webhook from before it still loses.
    people: Option<People>,
    updated_at: OffsetDateTime,
    expiry: Instant,
}

impl PeopleCache {
    pub fn new(ttl: Duration) -> PeopleCache {
        PeopleCache {
            entries: Mutex::new(HashMap::new()),
            ttl,
        }
    }

    pub fn get(&self, repo: &github::Repo, number: u32) -> Option<People> {
        let entries = self.entries.lock().unwrap();
        let entry = entries.get(&key(repo, number))?;
        if Instant::now() >= entry.expiry {
            return None;
        }
        entry.people.clone()
    }

    // Webhooks arrive in any order, so the PR GitHub updated last wins. Its
    // `updated_at` has only second precision: two changes in the same second
    // can't be ordered, so their people are forgotten for the next read to look
    // up. Whatever arrives next for that second is stored, even a lookup from a
    // replica that hasn't caught up.
    pub fn update(&self, repo: &github::Repo, pull_request: &github::PullRequest) {
        let key = key(repo, pull_request.number);
        let mut entries = self.entries.lock().unwrap();
        let updated_at = pull_request
            .updated_at
            .as_deref()
            .and_then(|t| OffsetDateTime::parse(t, &Rfc3339).ok());
        let Some(updated_at) = updated_at else {
            entries.remove(&key);
            return;
        };
        let people = People::new(pull_request);
        let expiry = Instant::now() + self.ttl;

        if let Some(entry) = entries.get_mut(&key) {
            if updated_at < entry.updated_at {
                return;
            }
            if updated_at == entry.updated_at {
                match entry.people {
                    Some(ref known) if *known != people => entry.people = None,
                    _ => entry.people = Some(people),
                }
                entry.expiry = expiry;
                return;
            }
        }
        entries.insert(
            key,
            Entry {
                people: Some(people),
                updated_at,
                expiry,
            },
        );
    }

    pub fn clean(&self) {
        let now = Instant::now();
        self.entries
            .lock()
            .unwrap()
            .retain(|_, entry| now < entry.expiry);
    }

    pub fn entry_count(&self) -> usize {
        self.entries.lock().unwrap().len()
    }
}

fn key(repo: &github::Repo, number: u32) -> (String, u32) {
    (repo.full_name.clone(), number)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TTL: Duration = Duration::from_secs(3600);

    fn repo() -> github::Repo {
        github::Repo::parse("http://the-github-host/some-user/some-repo").unwrap()
    }

    fn pr(updated_at: &str, assignee: &str) -> github::PullRequest {
        let mut pr = github::PullRequest::new();
        pr.number = 32;
        pr.user = github::User::new("the-pr-owner");
        pr.assignees = vec![github::User::new(assignee)];
        pr.updated_at = Some(updated_at.into());
        pr
    }

    fn assignee(cache: &PeopleCache) -> Option<String> {
        let people = cache.get(&repo(), 32)?;
        Some(people.assignees[0].login().to_string())
    }

    #[tokio::test(start_paused = true)]
    async fn test_keeps_the_newest() {
        let cache = PeopleCache::new(TTL);
        cache.update(&repo(), &pr("2026-11-13T10:00:01Z", "newer"));
        cache.update(&repo(), &pr("2026-11-13T10:00:00Z", "older"));
        assert_eq!(Some("newer".into()), assignee(&cache));

        cache.update(&repo(), &pr("2026-11-13T10:00:02Z", "newest"));
        assert_eq!(Some("newest".into()), assignee(&cache));
        assert_eq!(None, cache.get(&repo(), 33));
    }

    #[tokio::test(start_paused = true)]
    async fn test_drops_a_tie_it_cannot_order() {
        let cache = PeopleCache::new(TTL);
        cache.update(&repo(), &pr("2026-11-13T10:00:00Z", "first"));
        cache.update(&repo(), &pr("2026-11-13T10:00:00Z", "first"));
        assert_eq!(Some("first".into()), assignee(&cache));

        cache.update(&repo(), &pr("2026-11-13T10:00:00Z", "second"));
        assert_eq!(None, assignee(&cache));

        cache.update(&repo(), &pr("2026-11-13T09:59:59Z", "earlier"));
        assert_eq!(None, assignee(&cache));

        // Whatever arrives next for that second is the only thing known.
        cache.update(&repo(), &pr("2026-11-13T10:00:00Z", "first"));
        assert_eq!(Some("first".into()), assignee(&cache));
    }

    #[tokio::test(start_paused = true)]
    async fn test_drops_a_pr_without_updated_at() {
        let cache = PeopleCache::new(TTL);
        cache.update(&repo(), &pr("2026-11-13T10:00:00Z", "first"));
        let mut unordered = pr("", "second");
        unordered.updated_at = None;
        cache.update(&repo(), &unordered);
        assert_eq!(None, assignee(&cache));
    }

    #[tokio::test(start_paused = true)]
    async fn test_expires_and_cleans() {
        let cache = PeopleCache::new(TTL);
        cache.update(&repo(), &pr("2026-11-13T10:00:00Z", "first"));
        tokio::time::advance(TTL / 2).await;
        // A matching webhook confirms the entry is still current.
        cache.update(&repo(), &pr("2026-11-13T10:00:00Z", "first"));
        tokio::time::advance(TTL / 2).await;
        assert_eq!(Some("first".into()), assignee(&cache));

        tokio::time::advance(TTL / 2).await;
        assert_eq!(None, assignee(&cache));
        cache.clean();
        assert_eq!(0, cache.entry_count());
    }
}
