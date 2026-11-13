use std::sync::Arc;

use log::{error, info};

use crate::diffs::DiffOfDiffs;
use crate::git::Git;
use crate::git_clone_manager::GitCloneManager;
use crate::worker;
use octobot_lib::errors::*;
use octobot_lib::github;
use octobot_lib::github::Commit;
use octobot_lib::github::api::GithubSessionFactory;
use octobot_lib::metrics::{self, Metrics};

pub async fn comment_force_push(
    diffs: Result<DiffOfDiffs>,
    github: &dyn github::api::Session,
    owner: &str,
    repo: &str,
    pull_request: &github::PullRequest,
    before_hash: &str,
    after_hash: &str,
) -> Result<()> {
    let mut comment = format!(
        "Force-push detected: before: {}, after: {}: ",
        Commit::short_hash_str(before_hash),
        Commit::short_hash_str(after_hash)
    );

    let identical_diff;
    match diffs {
        Ok(ref diffs) => {
            if diffs.are_equal() {
                comment += "Identical diff post-rebase.";
                identical_diff = true;
            } else {
                comment += "Diff changed post-rebase.";
                let different_files = diffs.different_patch_files();
                if !different_files.is_empty() {
                    comment += "\n\nChanged files:\n";
                    for file in different_files {
                        comment += &format!("* {}\n", file.path());
                    }
                }

                identical_diff = false;
            }
        }
        Err(e) => {
            comment += "Unable to calculate diff.";
            identical_diff = false;
            error!("Error calculating force push diff: {}", e);
        }
    };

    let mut reapprove = false;

    if identical_diff {
        if let Some(review_msg) =
            dismissed_approval(github, owner, repo, pull_request, before_hash, after_hash).await
        {
            reapprove = true;
            let msg = format!(
                "{}\n\nReapproved based on review by {}",
                comment, review_msg
            );
            if let Err(e) = github
                .approve_pull_request(owner, repo, pull_request.number, after_hash, Some(&msg))
                .await
            {
                error!(
                    "Error reapproving pull request #{}: {}",
                    pull_request.number, e
                );
            }
        }
    }

    // Only comment if not reapproved since reapproval already includes the "identical diff" comment.
    if !reapprove {
        if let Err(e) = github
            .comment_pull_request(owner, repo, pull_request.number, &comment)
            .await
        {
            error!("Error sending github PR comment: {}", e);
        }
    }

    Ok(())
}

// Who approved `before_hash`, if this push's dismissal of that approval is the
// PR's latest dismissal. Lookup failures mean no reapproval, never a failed push.
async fn dismissed_approval(
    github: &dyn github::api::Session,
    owner: &str,
    repo: &str,
    pull_request: &github::PullRequest,
    before_hash: &str,
    after_hash: &str,
) -> Option<String> {
    // The reviews are usually one page and the timeline many, and only a dismissed
    // review of the old head can be the approval to restore.
    let reviews = match github
        .get_pull_request_reviews(owner, repo, pull_request.number)
        .await
    {
        Ok(reviews) => reviews,
        Err(e) => {
            error!(
                "Error fetching reviews for PR #{}: {}",
                pull_request.number, e
            );
            return None;
        }
    };
    if !reviews
        .iter()
        .any(|r| r.state == "DISMISSED" && r.commit_id.as_deref() == Some(before_hash))
    {
        return None;
    }

    let timeline = match github
        .get_timeline_newest_first(owner, repo, pull_request.number, &|events| {
            find_reapproval(events, before_hash, after_hash) != Reapproval::Undecided
        })
        .await
    {
        Ok(timeline) => timeline,
        Err(e) => {
            error!(
                "Error fetching timeline for PR #{}: {}",
                pull_request.number, e
            );
            return None;
        }
    };

    match find_reapproval(&timeline, before_hash, after_hash) {
        Reapproval::Review { id, reviewer } => {
            info!(
                "Reapproving PR {}/{} #{} based on review #{}",
                owner, repo, pull_request.number, id,
            );
            Some(reviewer)
        }
        Reapproval::No | Reapproval::Undecided => None,
    }
}

#[derive(Debug, PartialEq)]
enum Reapproval {
    // The events so far, newest first, end before the answer.
    Undecided,
    No,
    Review { id: u32, reviewer: String },
}

// Only the latest dismissal counts: it must be this push dismissing an approval,
// and that approval must have reviewed `before_hash`.
fn find_reapproval(
    events_newest_first: &[github::TimelineEvent],
    before_hash: &str,
    after_hash: &str,
) -> Reapproval {
    let mut approval_review_id = None;
    for event in events_newest_first {
        match approval_review_id {
            None if event.is_review_dismissal() => {
                if !event.is_review_dismissal_for(after_hash) {
                    return Reapproval::No;
                }
                match event.dismissed_review_id() {
                    Some(id) => approval_review_id = Some(id),
                    None => return Reapproval::No,
                }
            }
            Some(id) if event.is_review_for(id, before_hash) => {
                return Reapproval::Review {
                    id,
                    reviewer: event.review_user_message(id),
                };
            }
            _ => {}
        }
    }
    Reapproval::Undecided
}

pub async fn diff_force_push(
    github: &dyn github::api::Session,
    clone_mgr: &GitCloneManager,
    owner: &str,
    repo: &str,
    pull_request: &github::PullRequest,
    before_hash: &str,
    after_hash: &str,
) -> Result<DiffOfDiffs> {
    let held_clone_dir = clone_mgr.clone(owner, repo).await?;
    let clone_dir = held_clone_dir.dir();

    let git = Git::new(github.github_host(), github.github_token(), clone_dir);

    // It is important to get the local branch up to date for `find_base_branch_commit`
    let base_branch = &pull_request.base.ref_name;
    git.checkout_branch(base_branch, &format!("origin/{}", base_branch))?;

    // create a branch for the before hash then fetch, then delete it to get the ref
    let temp_branch = format!("octobot-{}-{}", pull_request.head.ref_name, before_hash);
    github
        .create_branch(owner, repo, &temp_branch, before_hash)
        .await?;
    git.run(&["fetch"])?;
    github.delete_branch(owner, repo, &temp_branch).await?;

    // find the first commits in base_branch that `before`/`after` came from
    let before_base_commit = git.find_base_branch_commit(before_hash, base_branch)?;
    let after_base_commit = git.find_base_branch_commit(after_hash, base_branch)?;

    let before_diff = git.diff(&before_base_commit, before_hash)?;
    let after_diff = git.diff(&after_base_commit, after_hash)?;

    Ok(DiffOfDiffs::new(&before_diff, &after_diff))
}

#[derive(Debug, PartialEq)]
pub struct ForcePushRequest {
    pub repo: github::Repo,
    pub pull_request: github::PullRequest,
    pub before_hash: String,
    pub after_hash: String,
}

struct Runner {
    github_app: Arc<dyn GithubSessionFactory>,
    clone_mgr: Arc<GitCloneManager>,
    metrics: Arc<Metrics>,
}

pub fn req(
    repo: &github::Repo,
    pull_request: &github::PullRequest,
    before_hash: &str,
    after_hash: &str,
) -> ForcePushRequest {
    ForcePushRequest {
        repo: repo.clone(),
        pull_request: pull_request.clone(),
        before_hash: before_hash.to_string(),
        after_hash: after_hash.to_string(),
    }
}

pub fn new_runner(
    github_app: Arc<dyn GithubSessionFactory>,
    clone_mgr: Arc<GitCloneManager>,
    metrics: Arc<Metrics>,
) -> Arc<dyn worker::Runner<ForcePushRequest>> {
    Arc::new(Runner {
        github_app,
        clone_mgr,
        metrics,
    })
}

#[async_trait::async_trait]
impl worker::Runner<ForcePushRequest> for Runner {
    async fn handle(&self, req: ForcePushRequest) {
        let _scoped_count = metrics::scoped_inc(&self.metrics.current_force_push_count);
        let _scoped_timer = self.metrics.force_push_duration.start_timer();

        let github = match self
            .github_app
            .new_session(req.repo.owner.login(), &req.repo.name)
            .await
        {
            Ok(g) => g,
            Err(e) => {
                error!("Error getting new session: {}", e);
                return;
            }
        };

        let diffs = diff_force_push(
            &github,
            &self.clone_mgr,
            req.repo.owner.login(),
            &req.repo.name,
            &req.pull_request,
            &req.before_hash,
            &req.after_hash,
        )
        .await;

        let comment = comment_force_push(
            diffs,
            &github,
            req.repo.owner.login(),
            &req.repo.name,
            &req.pull_request,
            &req.before_hash,
            &req.after_hash,
        )
        .await;
        if let Err(e) = comment {
            error!("Error diffing force push: {}", e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use github::{DismissedReview, TimelineEvent, User};

    const BEFORE: &str = "before-hash";
    const AFTER: &str = "after-hash";

    fn approval() -> TimelineEvent {
        TimelineEvent::new_review(BEFORE, 1234, User::new("joe"), "http://the-review")
    }

    fn push_dismissal() -> TimelineEvent {
        TimelineEvent::new_dismissed_review(DismissedReview::by_commit("approved", AFTER, 1234))
    }

    #[test]
    fn test_find_reapproval_needs_older_events_until_the_approval() {
        let newest_first = vec![TimelineEvent::new("labeled"), push_dismissal()];
        assert_eq!(
            Reapproval::Undecided,
            find_reapproval(&newest_first, BEFORE, AFTER)
        );
    }

    #[test]
    fn test_find_reapproval_needs_older_events_until_a_dismissal() {
        let newest_first = vec![TimelineEvent::new("labeled")];
        assert_eq!(
            Reapproval::Undecided,
            find_reapproval(&newest_first, BEFORE, AFTER)
        );
    }

    #[test]
    fn test_find_reapproval_finds_the_dismissed_approval() {
        let newest_first = vec![push_dismissal(), TimelineEvent::new("labeled"), approval()];
        assert_eq!(
            Reapproval::Review {
                id: 1234,
                reviewer: "[joe](http://the-review)".into()
            },
            find_reapproval(&newest_first, BEFORE, AFTER)
        );
    }

    #[test]
    fn test_find_reapproval_stops_at_a_later_unrelated_dismissal() {
        let newest_first = vec![
            TimelineEvent::new_dismissed_review(DismissedReview::by_user("approved", "nope")),
            push_dismissal(),
            approval(),
        ];
        assert_eq!(
            Reapproval::No,
            find_reapproval(&newest_first, BEFORE, AFTER)
        );
    }
}
