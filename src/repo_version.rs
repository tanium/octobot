use std::borrow::Borrow;
use std::path::Path;
use std::sync::Arc;

#[cfg(target_os="linux")]
use std::process::{Command, Stdio};

use threadpool::ThreadPool;

use config::{Config, JiraConfig};
use git::Git;
use github;
use git_clone_manager::GitCloneManager;
use jira;
use worker;

pub fn comment_repo_version(version_script: &str,
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

// Only run version scripts on Linux since firejail is only for Linux and it doesn't
// seem like a good idea to allow generic code execution without any containerization.
#[cfg(not(target_os="linux"))]
fn run_script(_: &str, _: &Path) -> Result<String, String> {
    return Err("Version scripts only supported when running Linux.".into());
}

#[cfg(target_os="linux")]
fn run_script(version_script: &str, clone_dir: &Path) -> Result<String, String> {
    debug!("Running version script: {}", version_script);
    let cmd = Command::new("firejail")
        .arg("--overlay-tmpfs")
        .arg("--quiet")
        .arg("--private=.")
        .arg("--private-etc=hostname")
        .arg("--net=none")
        .arg("--private-tmp")
        .arg("--private-dev")
        .arg("-c")
        .arg("bash")
        .arg("-c")
        .arg(version_script)
        .current_dir(clone_dir)
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn();

    let child = match cmd {
        Ok(c) => c,
        Err(e) => return Err(format!("Error starting version script (script: {}): {}", version_script, e)),
    };

    let result = match child.wait_with_output() {
        Ok(r) => r,
        Err(e) => return Err(format!("Error running version script (script: {}): {}", version_script, e)),
    };

    let mut output = String::new();
    if result.stdout.len() > 0 {
        output += String::from_utf8_lossy(&result.stdout).as_ref();
        // skip over firejail output (even with --quiet)
        if output.starts_with("OverlayFS") {
            let new_lines: Vec<String> =
                output.lines().skip(1)
                              .skip_while(|s| s.trim().len() == 0)
                              .map(|s| s.into())
                              .collect();
            output = new_lines.join("\n");
        }

    }

    if !result.status.success() {
        if result.stderr.len() > 0 {
            output += String::from_utf8_lossy(&result.stderr).as_ref();
        }
        Err(format!("Error running version script (exit code {}; script: {}):\n{}",
                    result.status.code().unwrap_or(-1),
                    version_script,
                    output))
    } else {

        Ok(output.trim().to_string())
    }
}

#[derive(Debug)]
pub struct RepoVersionRequest {
    pub repo: github::Repo,
    pub branch: String,
    pub commit_hash: String,
    pub commits: Vec<github::PushCommit>,
}

struct Runner {
    config: Arc<Config>,
    github_session: Arc<github::api::Session>,
    jira_session: Option<Arc<jira::api::Session>>,
    clone_mgr: Arc<GitCloneManager>,
    thread_pool: ThreadPool,
}

pub fn req(repo: &github::Repo, branch: &str, commit_hash: &str, commits: &Vec<github::PushCommit>) -> RepoVersionRequest {
    RepoVersionRequest {
        repo: repo.clone(),
        branch: branch.to_string(),
        commit_hash: commit_hash.to_string(),
        commits: commits.clone(),
    }
}

pub fn new_worker(max_concurrency: usize,
                  config: Arc<Config>,
                  github_session: Arc<github::api::Session>,
                  jira_session: Option<Arc<jira::api::Session>>,
                  clone_mgr: Arc<GitCloneManager>)
                   -> worker::Worker<RepoVersionRequest> {
    worker::Worker::new(Runner {
        config: config,
        github_session: github_session,
        jira_session: jira_session,
        clone_mgr: clone_mgr,
        thread_pool: ThreadPool::new(max_concurrency),
    })
}

impl worker::Runner<RepoVersionRequest> for Runner {
    fn handle(&self, req: RepoVersionRequest) {
        let github_session = self.github_session.clone();
        let jira_session = self.jira_session.clone();
        let clone_mgr = self.clone_mgr.clone();
        let config = self.config.clone();

        // launch another thread to do the version calculation
        self.thread_pool.execute(move || {
            let version_script;
            {
                let repos_lock = config.repos();
                version_script = repos_lock.version_script(&req.repo);
            }

            if let Some(version_script) = version_script {
                if let Some(ref jira_session) = jira_session {
                    if let Some(ref jira_config) = config.jira {
                        let jira = jira_session.borrow();
                        if let Err(e) = comment_repo_version(&version_script,
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

#[cfg(target_os="linux")]
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;

    extern crate tempdir;
    use self::tempdir::TempDir;

    #[test]
    fn test_run_script() {
        let dir = TempDir::new("repo_version.rs").expect("create temp dir for repo_version.rs test");

        let sub_dir = dir.path().join("subdir");
        fs::create_dir(&sub_dir).expect("create subdir");

        let script_file = sub_dir.join("version.sh");
        {
            let mut file = fs::File::create(&script_file).expect("create file");
            file.write_all(b"echo 1.2.3.4").expect("write file");
        }

        assert_eq!(Ok("1.2.3.4".into()), run_script("bash version.sh", &sub_dir));
    }

    #[test]
    fn test_run_script_isolation() {
        let dir = TempDir::new("repo_version.rs").expect("create temp dir for repo_version.rs test");

        let parent_file = dir.path().join("private.txt");
        {
            let mut file = fs::File::create(&parent_file).expect("create file");
            file.write_all(b"I am a file").expect("write file");
        }

        let sub_dir = dir.path().join("subdir");
        fs::create_dir(&sub_dir).expect("create subdir");

        let script_file = sub_dir.join("version.sh");
        {
            let mut file = fs::File::create(&script_file).expect("create file");
            file.write_all(br#"
            # no `set -e` on purpose: try to do them all!
            rm ../private.txt
            rm version.sh
            touch ../muahaha.txt
            touch muahaha.txt

            echo 1.2.3.4
            "#).expect("write file");
        }

        assert_eq!(Ok("1.2.3.4".into()), run_script("bash version.sh", &sub_dir));

        assert!(parent_file.exists(), "version scripts should not be able to delete files outside its directory");
        assert!(script_file.exists(), "version scripts should not be able to delete files inside its directory");
        assert!(!dir.path().join("muahaha.txt").exists(), "version scripts should not be able to create files outside its directory");
        assert!(!sub_dir.join("muahaha.txt").exists(), "version scripts should not be able to create files inside its directory");
    }
}
