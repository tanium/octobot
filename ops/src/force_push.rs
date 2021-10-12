use std::sync::Arc;

use log::{error, info};

use crate::diffs::DiffOfDiffs;
use crate::git::Git;
use crate::git_clone_manager::GitCloneManager;
use crate::worker;
use octobot_lib::errors::*;
use octobot_lib::github;
use octobot_lib::github::api::GithubSessionFactory;
use octobot_lib::github::Commit;
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
                if different_files.len() > 0 {
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
        // Avoid failing the whole function if this fails
        let timeline = match github.get_timeline(owner, repo, pull_request.number).await {
            Ok(t) => t,
            Err(e) => {
                error!(
                    "Error fetching timeline for PR #{}: {}",
                    pull_request.number, e
                );
                vec![]
            }
        };

        let mut approval_review_id = None;
        let mut found_dismiss = false;
        let mut review_msg = String::new();

        for event in timeline.iter().rev() {
            // Look for first dismissal
            if !found_dismiss && event.is_review_dismissal() {
                if event.is_review_dismissal_for(after_hash) {
                    approval_review_id = event.dismissed_review_id();
                }
                found_dismiss = true;
                continue;
            }

            // Look for the dismissed approval
            if let Some(review_id) = approval_review_id {
                if event.is_review_for(review_id, before_hash) {
                    review_msg = event.review_user_message(review_id);
                    reapprove = true;
                    info!(
                        "Reapproving PR {}/{} #{} based on review #{:?}",
                        owner, repo, pull_request.number, review_id,
                    );
                    break;
                }
            }
        }

        if reapprove {
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
        github_app: github_app,
        clone_mgr: clone_mgr,
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
            .new_session(&req.repo.owner.login(), &req.repo.name)
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
            &req.repo.owner.login(),
            &req.repo.name,
            &req.pull_request,
            &req.before_hash,
            &req.after_hash,
        )
        .await;

        let comment = comment_force_push(
            diffs,
            &github,
            &req.repo.owner.login(),
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
