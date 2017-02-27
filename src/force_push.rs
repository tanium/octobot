use std::borrow::Borrow;
use std::sync::{Arc, Mutex};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread::{self, JoinHandle};

use threadpool::ThreadPool;

use config::Config;
use git::Git;
use github;
use github::Commit;
use git_clone_manager::GitCloneManager;

pub fn comment_force_push(diffs: Result<(String, String), String>,
                          github: &github::api::Session,
                          owner: &str,
                          repo: &str,
                          pull_request: &github::PullRequest,
                          before_hash: &str,
                          after_hash: &str) -> Result<(), String> {
    let mut comment = format!("Force-push detected: before: {}, after: {}: ",
                              Commit::short_hash_str(before_hash), Commit::short_hash_str(after_hash));
    match diffs {
        Ok(ref diffs) => {
            if diffs.0 == diffs.1 {
                comment += "Identical diff post-rebase";
            } else {
                // TODO: How to expose this diff -- maybe create a secret gist?
                // But that may raise permissions concerns for users who can read octobot's gists,
                // but perhaps not the original repo...
                comment += "Diff changed post-rebase";
            }
        },
        Err(e) => {
            comment += "Unable to calculate diff";
            error!("Error calculating force push diff: {}", e);
        }
    };

    if let Err(e) = github.comment_pull_request(owner, repo, pull_request.number, &comment) {
        error!("Error sending github PR comment: {}", e);
    }

    // TODO: reapply statuses if no change in force-push.

    Ok(())
}

pub fn diff_force_push(github: &github::api::Session,
                       clone_mgr: &GitCloneManager,
                       owner: &str,
                       repo: &str,
                       pull_request: &github::PullRequest,
                       before_hash: &str,
                       after_hash: &str) -> Result<(String, String), String> {
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

    Ok((before_diff, after_diff))
}

#[derive(Debug)]
pub enum ForcePushMessage {
    Stop,
    Check(ForcePushRequest),
}

#[derive(Debug)]
pub struct ForcePushRequest {
    pub repo: github::Repo,
    pub pull_request: github::PullRequest,
    pub before_hash: String,
    pub after_hash: String,
}

pub struct Worker {
    sender: Mutex<Sender<ForcePushMessage>>,
    handle: Option<JoinHandle<()>>,
}

impl ForcePushMessage {
    pub fn check(repo: &github::Repo, pull_request: &github::PullRequest, before_hash: &str, after_hash: &str) -> ForcePushMessage {
        ForcePushMessage::Check(ForcePushRequest {
            repo: repo.clone(),
            pull_request: pull_request.clone(),
            before_hash: before_hash.to_string(),
            after_hash: after_hash.to_string(),
        })
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let sender = self.new_sender();
        match sender.send(ForcePushMessage::Stop) {
            Ok(_) => {
                match self.handle.take().unwrap().join() {
                    Ok(_) => (),
                    Err(e) => error!("Error shutting down worker: {:?}", e),
                }
            }
            Err(e) => error!("Error sending stop message: {}", e),
        }
    }
}

impl Worker {
    pub fn new(max_concurrency: usize,
               config: Arc<Config>,
               github_session: Arc<github::api::Session>,
               clone_mgr: Arc<GitCloneManager>)
               -> Worker {
        let (tx, rx) = channel();

        Worker {
            sender: Mutex::new(tx),
            handle: Some(thread::spawn(move || {
                let runner = WorkerRunner {
                    rx: rx,
                    config: config,
                    github_session: github_session,
                    clone_mgr: clone_mgr.clone(),
                    thread_pool: ThreadPool::new(max_concurrency),
                };
                runner.run();
            })),
        }
    }

    pub fn new_sender(&self) -> Sender<ForcePushMessage> {
        let sender = self.sender.lock().unwrap();
        sender.clone()
    }
}

struct WorkerRunner {
    rx: Receiver<ForcePushMessage>,
    config: Arc<Config>,
    github_session: Arc<github::api::Session>,
    clone_mgr: Arc<GitCloneManager>,
    thread_pool: ThreadPool,
}

impl WorkerRunner {
    fn run(&self) {
        loop {
            match self.rx.recv() {
                Ok(ForcePushMessage::Stop) => break,
                Ok(ForcePushMessage::Check(req)) => self.handle_check(req),
                Err(e) => error!("Error receiving message: {}", e),
            };
        }
    }

    fn handle_check(&self, req: ForcePushRequest) {
        let github_session = self.github_session.clone();
        let clone_mgr = self.clone_mgr.clone();
        let _ = self.config.clone(); // TODO

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
