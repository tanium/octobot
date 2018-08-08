use std::sync::Arc;

use config::Config;
use diffs::DiffOfDiffs;
use errors::*;
use git::Git;
use git_clone_manager::GitCloneManager;
use github;
use github::Commit;
use github::api::GithubSessionFactory;
use worker;

pub fn comment_force_push(
    diffs: Result<DiffOfDiffs>,
    reapply_statuses: Vec<String>,
    github: &github::api::Session,
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
        let statuses = match github.get_statuses(owner, repo, before_hash) {
            Ok(t) => t,
            Err(e) => {
                error!("Error fetching statuses for PR #{}: {}", pull_request.number, e);
                vec![]
            }
        };

        // keep track of seen statuses to only track latest ones
        let mut seen = vec![];
        for status in &statuses {
            if let Some(ref context) = status.context {
                if seen.contains(&context) {
                    continue;
                }
                seen.push(context.into());

                if reapply_statuses.contains(&context) {
                    let mut new_status = status.clone();
                    new_status.creator = None;
                    let octobot_was_here = "(reapplied by octobot)";
                    if let Some(ref mut desc) = new_status.description {
                        desc.push_str(" ");
                        desc.push_str(octobot_was_here);
                    } else {
                        new_status.description = Some(octobot_was_here.into());
                    }

                    if let Err(e) = github.create_status(owner, repo, after_hash, &new_status) {
                        error!("Error re-applying status to new commit {}, {:?}: {}", after_hash, new_status, e);
                    }
                }
            }
        }

        // Avoid failing the whole function if this fails
        let timeline = match github.get_timeline(owner, repo, pull_request.number) {
            Ok(t) => t,
            Err(e) => {
                error!("Error fetching timeline for PR #{}: {}", pull_request.number, e);
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
                        owner,
                        repo,
                        pull_request.number,
                        review_id,
                    );
                    break;
                }
            }
        }

        if reapprove {
            let msg = format!("{}\n\nReapproved based on review by {}", comment, review_msg);
            if let Err(e) = github.approve_pull_request(owner, repo, pull_request.number, after_hash, Some(&msg)) {
                error!("Error reapproving pull request #{}: {}", pull_request.number, e);
            }
        }
    }

    // Only comment if not reapproved since reapproval already includes the "identical diff" comment.
    if !reapprove {
        if let Err(e) = github.comment_pull_request(owner, repo, pull_request.number, &comment) {
            error!("Error sending github PR comment: {}", e);
        }
    }

    Ok(())
}

pub fn diff_force_push(
    github: &github::api::Session,
    clone_mgr: &GitCloneManager,
    owner: &str,
    repo: &str,
    pull_request: &github::PullRequest,
    before_hash: &str,
    after_hash: &str,
) -> Result<DiffOfDiffs> {
    let held_clone_dir = clone_mgr.clone(owner, repo)?;
    let clone_dir = held_clone_dir.dir();

    let git = Git::new(github.github_host(), github.github_token(), clone_dir);

    // It is important to get the local branch up to date for `find_base_branch_commit`
    let base_branch = &pull_request.base.ref_name;
    git.checkout_branch(base_branch, &format!("origin/{}", base_branch))?;

    // create a branch for the before hash then fetch, then delete it to get the ref
    let temp_branch = format!("octobot-{}-{}", pull_request.head.ref_name, before_hash);
    github.create_branch(owner, repo, &temp_branch, before_hash)?;
    git.run(&["fetch"])?;
    github.delete_branch(owner, repo, &temp_branch)?;

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
    config: Arc<Config>,
    github_app: Arc<GithubSessionFactory>,
    clone_mgr: Arc<GitCloneManager>,
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
    config: Arc<Config>,
    github_app: Arc<GithubSessionFactory>,
    clone_mgr: Arc<GitCloneManager>,
) -> Arc<worker::Runner<ForcePushRequest>> {
    Arc::new(Runner {
        config: config,
        github_app: github_app,
        clone_mgr: clone_mgr,
    })
}

impl worker::Runner<ForcePushRequest> for Runner {
    fn handle(&self, req: ForcePushRequest) {
        let statuses = self.config.repos().force_push_reapply_statuses(&req.repo);

        let github = match self.github_app.new_session(&req.repo.owner.login(), &req.repo.name) {
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
        );

        let comment = comment_force_push(
            diffs,
            statuses,
            &github,
            &req.repo.owner.login(),
            &req.repo.name,
            &req.pull_request,
            &req.before_hash,
            &req.after_hash,
        );
        if let Err(e) = comment {
            error!("Error diffing force push: {}", e);
        }
    }
}
