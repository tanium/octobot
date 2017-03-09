use std::borrow::Borrow;
use std::sync::Arc;

use threadpool::ThreadPool;

use config::Config;
use diffs::DiffOfDiffs;
use git::Git;
use github;
use github::Commit;
use git_clone_manager::GitCloneManager;
use worker;

pub fn comment_force_push(diffs: Result<DiffOfDiffs, String>,
                          reapply_statuses: Vec<String>,
                          github: &github::api::Session,
                          owner: &str,
                          repo: &str,
                          pull_request: &github::PullRequest,
                          before_hash: &str,
                          after_hash: &str) -> Result<(), String> {
    let mut comment = format!("Force-push detected: before: {}, after: {}: ",
                              Commit::short_hash_str(before_hash), Commit::short_hash_str(after_hash));

    let identical_diff;
    match diffs {
        Ok(ref diffs) => {
            if diffs.are_equal() {
                comment += "Identical diff post-rebase";
                identical_diff = true;
            } else {
                comment += "Diff changed post-rebase";
                let different_files = diffs.different_patch_files();
                if different_files.len() > 0 {
                    comment += "\n\nChanged files:\n";
                    for file in different_files {
                        comment += &format!("* {}\n", file.path());
                    }
                }

                identical_diff = false;
            }
        },
        Err(e) => {
            comment += "Unable to calculate diff";
            identical_diff = false;
            error!("Error calculating force push diff: {}", e);
        }
    };

    if let Err(e) = github.comment_pull_request(owner, repo, pull_request.number, &comment) {
        error!("Error sending github PR comment: {}", e);
    }

    if identical_diff {
        let statuses = match github.get_statuses(owner, repo, before_hash) {
            Ok(s) => s,
            Err(e) => return Err(format!("Error looking up github statuses: {}",e ))
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
    }

    Ok(())
}

pub fn diff_force_push(github: &github::api::Session,
                       clone_mgr: &GitCloneManager,
                       owner: &str,
                       repo: &str,
                       pull_request: &github::PullRequest,
                       before_hash: &str,
                       after_hash: &str) -> Result<DiffOfDiffs, String> {
    let held_clone_dir = try!(clone_mgr.clone(owner, repo));
    let clone_dir = held_clone_dir.dir();

    let git = Git::new(github.github_host(), github.github_token(), clone_dir);

    // It is important to get the local branch up to date for `find_base_branch_commit`
    let base_branch = &pull_request.base.ref_name;
    try!(git.checkout_branch(base_branch, &format!("origin/{}", base_branch)));

    // create a branch for the before hash then fetch, then delete it to get the ref
    let temp_branch = format!("octobot-{}-{}", pull_request.head.ref_name, before_hash);
    try!(github.create_branch(owner, repo, &temp_branch, before_hash));
    try!(git.run(&["fetch"]));
    try!(github.delete_branch(owner, repo, &temp_branch));

    // find the first commits in base_branch that `before`/`after` came from
    let before_base_commit = try!(git.find_base_branch_commit(before_hash, base_branch));
    let after_base_commit = try!(git.find_base_branch_commit(after_hash, base_branch));

    let before_diff = try!(git.diff(&before_base_commit, before_hash));
    let after_diff = try!(git.diff(&after_base_commit, after_hash));

    Ok(DiffOfDiffs::new(&before_diff, &after_diff))
}

#[derive(Debug)]
pub struct ForcePushRequest {
    pub repo: github::Repo,
    pub pull_request: github::PullRequest,
    pub before_hash: String,
    pub after_hash: String,
}

struct Runner {
    config: Arc<Config>,
    github_session: Arc<github::api::Session>,
    clone_mgr: Arc<GitCloneManager>,
    thread_pool: ThreadPool,
}


pub fn req(repo: &github::Repo, pull_request: &github::PullRequest, before_hash: &str, after_hash: &str) -> ForcePushRequest {
    ForcePushRequest {
        repo: repo.clone(),
        pull_request: pull_request.clone(),
        before_hash: before_hash.to_string(),
        after_hash: after_hash.to_string(),
    }
}

pub fn new_worker(max_concurrency: usize,
                  config: Arc<Config>,
                  github_session: Arc<github::api::Session>,
                  clone_mgr: Arc<GitCloneManager>)
                    -> worker::Worker<ForcePushRequest> {
    worker::Worker::new(Runner {
        config: config,
        github_session: github_session,
        clone_mgr: clone_mgr,
        thread_pool: ThreadPool::new(max_concurrency),
    })
}

impl worker::Runner<ForcePushRequest> for Runner {
    fn handle(&self, req: ForcePushRequest) {
        let github_session = self.github_session.clone();
        let clone_mgr = self.clone_mgr.clone();
        let config = self.config.clone();
        let statuses = config.repos().force_push_reapply_statuses(&req.repo);

        // launch another thread to do the version calculation
        self.thread_pool.execute(move || {
            let github = github_session.borrow();
            let diffs = diff_force_push(github,
                                        &clone_mgr,
                                        &req.repo.owner.login(),
                                        &req.repo.name,
                                        &req.pull_request,
                                        &req.before_hash,
                                        &req.after_hash);

            let comment = comment_force_push(diffs,
                                             statuses,
                                             github,
                                             &req.repo.owner.login(),
                                             &req.repo.name,
                                             &req.pull_request,
                                             &req.before_hash,
                                             &req.after_hash);
            if let Err(e) = comment {
                error!("Error diffing force push: {}", e);
            }
        });
    }
}
