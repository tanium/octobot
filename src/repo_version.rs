use std::borrow::Borrow;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread::{self, JoinHandle};

use threadpool::ThreadPool;

use config::{Config, JiraConfig};
use git::Git;
use github;
use git_clone_manager::GitCloneManager;
use jira;

pub fn comment_repo_version(version_script: &Vec<String>,
                            jira_config: &JiraConfig,
                            jira: &jira::api::Session,
                            github: &github::api::Session,
                            clone_mgr: &GitCloneManager,
                            owner: &str,
                            repo: &str,
                            branch_name: &str,
                            commit_hash: &str,
                            commits: &Vec<github::PushCommit>) -> Result<(), String> {
    let held_clone_dir = try!(clone_mgr.clone(owner, repo));
    let clone_dir = held_clone_dir.dir();

    let git = Git::new(github.github_host(), github.github_token(), &clone_dir);

    // setup branch
    try!(git.checkout_branch(branch_name, commit_hash));

    let version = try!(run_script(version_script, clone_dir));

    let maybe_version = if version.len() > 0 {
        Some(version.as_str())
    } else {
        None
    };

    // resolve with version
    jira::workflow::resolve_issue(branch_name, maybe_version, commits, jira, jira_config);

    Ok(())
}

fn run_script(version_script: &Vec<String>, clone_dir: &Path) -> Result<String, String> {
    debug!("Running version script: {:?}", version_script);
    let cmd = match version_script.iter().next() {
        Some(c) => c,
        None => return Err("Version script is empty!".into()),
    };
    let args: Vec<&String> = version_script.iter().skip(1).collect();

    let cmd = Command::new(cmd)
        .args(&args)
        .current_dir(clone_dir)
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn();

    let child = match cmd {
        Ok(c) => c,
        Err(e) => return Err(format!("Error starting version script: {}", e)),
    };

    let result = match child.wait_with_output() {
        Ok(r) => r,
        Err(e) => return Err(format!("Error running version script: {}", e)),
    };

    let mut output = String::new();
    if result.stdout.len() > 0 {
        output += String::from_utf8_lossy(&result.stdout).as_ref();
    }

    if !result.status.success() {
        if result.stderr.len() > 0 {
            output += String::from_utf8_lossy(&result.stderr).as_ref();
        }
        Err(format!("Error running version script (exit code {}):\n{}",
                    result.status.code().unwrap_or(-1),
                    output))
    } else {

        Ok(output.trim().to_string())
    }
}

#[derive(Debug)]
pub enum RepoVersionMessage {
    Stop,
    Version(RepoVersionRequest),
}

#[derive(Debug)]
pub struct RepoVersionRequest {
    pub repo: github::Repo,
    pub branch: String,
    pub commit_hash: String,
    pub commits: Vec<github::PushCommit>,
}

pub struct Worker {
    sender: Mutex<Sender<RepoVersionMessage>>,
    handle: Option<JoinHandle<()>>,
}

impl RepoVersionMessage {
    pub fn version(repo: &github::Repo, branch: &str, commit_hash: &str, commits: &Vec<github::PushCommit>) -> RepoVersionMessage {
        RepoVersionMessage::Version(RepoVersionRequest {
            repo: repo.clone(),
            branch: branch.to_string(),
            commit_hash: commit_hash.to_string(),
            commits: commits.clone(),
        })
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let sender = self.new_sender();
        match sender.send(RepoVersionMessage::Stop) {
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
               jira_session: Option<Arc<jira::api::Session>>,
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
                    jira_session: jira_session,
                    clone_mgr: clone_mgr.clone(),
                    thread_pool: ThreadPool::new(max_concurrency),
                };
                runner.run();
            })),
        }
    }

    pub fn new_sender(&self) -> Sender<RepoVersionMessage> {
        let sender = self.sender.lock().unwrap();
        sender.clone()
    }
}

struct WorkerRunner {
    rx: Receiver<RepoVersionMessage>,
    config: Arc<Config>,
    github_session: Arc<github::api::Session>,
    jira_session: Option<Arc<jira::api::Session>>,
    clone_mgr: Arc<GitCloneManager>,
    thread_pool: ThreadPool,
}

impl WorkerRunner {
    fn run(&self) {
        loop {
            match self.rx.recv() {
                Ok(RepoVersionMessage::Stop) => break,
                Ok(RepoVersionMessage::Version(req)) => self.handle_version(req),
                Err(e) => error!("Error receiving message: {}", e),
            };
        }
    }

    fn handle_version(&self, req: RepoVersionRequest) {
        let github_session = self.github_session.clone();
        let jira_session = self.jira_session.clone();
        let clone_mgr = self.clone_mgr.clone();
        let config = self.config.clone();

        // launch another thread to do the version calculation
        self.thread_pool.execute(move || {
            if let Some(version_script) = config.repos.version_script(&req.repo) {
                if let Some(ref jira_session) = jira_session {
                    if let Some(ref jira_config) = config.jira {
                        let jira = jira_session.borrow();
                        if let Err(e) = comment_repo_version(version_script,
                                                             jira_config,
                                                             jira,
                                                             github_session.borrow(),
                                                             &clone_mgr,
                                                             &req.repo.owner.login(),
                                                             &req.repo.name,
                                                             &req.branch,
                                                             &req.commit_hash,
                                                             &req.commits) {
                            error!("Error running version script: {}", e);
                            // resolve the issue with no version
                            jira::workflow::resolve_issue(&req.branch, None, &req.commits, jira, jira_config);
                        }
                    }
                }
            }
        });
    }
}
