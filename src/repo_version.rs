use std::borrow::Borrow;
use std::path::Path;
use std::sync::Arc;

#[cfg(target_os = "linux")]
use std::process::{Command, Stdio};

use failure::format_err;
use log;
#[cfg(target_os = "linux")]
use log::debug;
use log::error;

use crate::config::{Config, JiraConfig};
use crate::errors::*;
use crate::git::Git;
use crate::git_clone_manager::GitCloneManager;
use crate::github;
use crate::github::api::{GithubSessionFactory, Session};
use crate::jira;
use crate::messenger;
use crate::slack::{SlackAttachmentBuilder, SlackRequest};
use crate::worker;

#[cfg(target_os = "linux")]
use crate::docker;

pub fn comment_repo_version(
    version_script: &str,
    jira_config: &JiraConfig,
    jira: &dyn jira::api::Session,
    github_app: &dyn GithubSessionFactory,
    clone_mgr: &GitCloneManager,
    owner: &str,
    repo: &str,
    branch_name: &str,
    commit_hash: &str,
    commits: &Vec<github::PushCommit>,
    jira_projects: &Vec<String>,
) -> Result<()> {
    let github = github_app.new_session(owner, repo)?;
    let held_clone_dir = clone_mgr.clone(owner, repo)?;
    let clone_dir = held_clone_dir.dir();

    let git = Git::new(github.github_host(), github.github_token(), &clone_dir);

    // setup branch
    git.checkout_branch(branch_name, commit_hash)?;

    let version = run_script(version_script, clone_dir)?;

    let maybe_version = if version.len() > 0 {
        Some(version.as_str())
    } else {
        None
    };

    // resolve with version
    jira::workflow::resolve_issue(branch_name, maybe_version, commits, jira_projects, jira, jira_config);

    jira::workflow::add_pending_version(maybe_version, commits, jira_projects, jira);

    Ok(())
}

// Only run version scripts on Linux since firejail is only for Linux and it doesn't
// seem like a good idea to allow generic code execution without any containerization.
#[cfg(not(target_os = "linux"))]
fn run_script(_: &str, _: &Path) -> Result<String> {
    return Err(format_err!("Version scripts only supported when running Linux."));
}

#[cfg(target_os = "linux")]
fn run_script(version_script: &str, clone_dir: &Path) -> Result<String> {
    debug!("Running version script: {}", version_script);
    let mut cmd = Command::new("firejail");
    cmd.arg("--quiet")
        .arg("--private=.")
        .arg("--private-etc=hostname,alternatives,firejail")
        .arg("--net=none")
        .arg("--private-tmp")
        .arg("--private-dev");

    if docker::in_docker() {
        // Otherwise we get "Warning: an existing sandbox was detected"
        // https://github.com/netblue30/firejail/issues/189
        cmd.arg("--force");
    }

    cmd.arg("-c")
        .arg("bash")
        .arg("-c")
        .arg(version_script)
        .current_dir(clone_dir)
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .stdout(Stdio::piped());

    log::info!("Running version script {:?} from {:?}", version_script, clone_dir);

    let child = cmd
        .spawn()
        .map_err(|e| format_err!("Error starting version script (script: {}): {}", version_script, e))?;
    let result = child
        .wait_with_output()
        .map_err(|e| format_err!("Error running version script (script: {}): {}", version_script, e))?;

    let mut output = String::new();
    if result.stdout.len() > 0 {
        let stdout = String::from_utf8_lossy(&result.stdout).into_owned();
        debug!("Version script stdout: \n---\n{}\n---", stdout);
        output += &stdout;
        // skip over firejail output (even with --quiet)
        if output.starts_with("OverlayFS") {
            let new_lines: Vec<String> = output
                .lines()
                .skip(1)
                .skip_while(|s| s.trim().len() == 0)
                .map(|s| s.into())
                .collect();
            output = new_lines.join("\n");
        }
    }

    let mut stderr = String::new();
    if result.stderr.len() > 0 {
        stderr = String::from_utf8_lossy(&result.stderr).into_owned();
        debug!("Version script stderr: \n---\n{}\n---", stderr);
    }

    let output = output.trim().to_string();

    // Note: there are some firejail failure conditions that do not trigger a non-zero exit code.
    // To catch these, and in the general case, we treat an empty version as an error.
    if !result.status.success() || output.is_empty() {
        Err(format_err!(
            "Error running version script (exit code {}; script: {}):\n{}\n{}",
            result.status.code().unwrap_or(-1),
            version_script,
            output,
            stderr
        ))
    } else {
        if !stderr.is_empty() {
            log::info!("Version script succeeded, but printed to stderr: {}", stderr);
        }

        Ok(output)
    }
}

#[derive(Debug, PartialEq)]
pub struct RepoVersionRequest {
    pub repo: github::Repo,
    pub branch: String,
    pub commit_hash: String,
    pub commits: Vec<github::PushCommit>,
}

struct Runner {
    config: Arc<Config>,
    github_app: Arc<dyn GithubSessionFactory>,
    jira_session: Option<Arc<dyn jira::api::Session>>,
    clone_mgr: Arc<GitCloneManager>,
    slack: Arc<dyn worker::Worker<SlackRequest>>,
}

pub fn req(
    repo: &github::Repo,
    branch: &str,
    commit_hash: &str,
    commits: &Vec<github::PushCommit>,
) -> RepoVersionRequest {
    RepoVersionRequest {
        repo: repo.clone(),
        branch: branch.to_string(),
        commit_hash: commit_hash.to_string(),
        commits: commits.clone(),
    }
}

pub fn new_runner(
    config: Arc<Config>,
    github_app: Arc<dyn GithubSessionFactory>,
    jira_session: Option<Arc<dyn jira::api::Session>>,
    clone_mgr: Arc<GitCloneManager>,
    slack: Arc<dyn worker::Worker<SlackRequest>>,
) -> Arc<dyn worker::Runner<RepoVersionRequest>> {
    Arc::new(Runner {
        config: config,
        github_app: github_app,
        jira_session: jira_session,
        clone_mgr: clone_mgr,
        slack: slack,
    })
}

impl worker::Runner<RepoVersionRequest> for Runner {
    fn handle(&self, req: RepoVersionRequest) {
        let configs;
        {
            let repos_lock = self.config.repos();
            configs = repos_lock.jira_configs(&req.repo, &req.branch);
        }

        if let Some(ref jira_session) = self.jira_session {
            if let Some(ref jira_config) = self.config.jira {
                for config in &configs {
                    // Don't run version scripts for jiras not mentioned
                    if !jira::workflow::references_jira(&req.commits, &config.jira_project) {
                        continue;
                    }

                    let mut resolved = false;
                    let jira = jira_session.borrow();
                    let jira_projects = vec![config.jira_project.clone()];

                    if !config.version_script.is_empty() {
                        if let Err(e) = comment_repo_version(
                            &config.version_script,
                            jira_config,
                            jira,
                            self.github_app.borrow(),
                            self.clone_mgr.borrow(),
                            &req.repo.owner.login(),
                            &req.repo.name,
                            &req.branch,
                            &req.commit_hash,
                            &req.commits,
                            &jira_projects,
                        ) {
                            error!("Error running version script {}: {}", config.version_script, e);
                            let messenger = messenger::new(self.config.clone(), self.slack.clone());

                            let attach = SlackAttachmentBuilder::new(&format!("{}", e))
                                .title(config.version_script.clone())
                                .color("danger")
                                .build();

                            messenger.send_to_channel(
                                &format!("Error running version script for [{}]", config.jira_project),
                                &vec![attach],
                                &req.repo,
                                &req.branch,
                                &req.commits,
                            );
                        } else {
                            resolved = true
                        }
                    }

                    // resolve the issue with no version if version script is missing or failed
                    if !resolved {
                        jira::workflow::resolve_issue(
                            &req.branch,
                            None,
                            &req.commits,
                            &jira_projects,
                            jira,
                            jira_config,
                        );
                    }
                }
            }
        }
    }
}

#[cfg(target_os = "linux")]
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;

    use tempdir::TempDir;

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

        assert_eq!("1.2.3.4", run_script("bash version.sh", &sub_dir).unwrap());
    }

    #[test]
    fn test_run_script_failure() {
        let dir = TempDir::new("repo_version.rs").expect("create temp dir for repo_version.rs test");

        let sub_dir = dir.path().join("subdir");
        fs::create_dir(&sub_dir).expect("create subdir");

        let script_file = sub_dir.join("version.sh");
        {
            let mut file = fs::File::create(&script_file).expect("create file");
            file.write_all(b"echo out-err; echo err-err >&2; exit 1")
                .expect("write file");
        }

        let err = format!("{}", run_script("bash version.sh", &sub_dir).unwrap_err());
        assert!(err.contains("Error running version script"), err);
        assert!(err.contains("out-err"), err);
        assert!(err.contains("err-err"), err);
    }

    #[test]
    fn test_run_script_failure_firejail_error() {
        let dir = TempDir::new("repo_version.rs").expect("create temp dir for repo_version.rs test");

        let sub_dir = dir.path().join("subdir");
        fs::create_dir(&sub_dir).expect("create subdir");

        let script_file = sub_dir.join("version.sh");
        {
            let mut file = fs::File::create(&script_file).expect("create file");
            file.write_all(b">&2 echo some error").expect("write file");
        }

        let err = format!("{}", run_script("bash version.sh", &sub_dir).unwrap_err());
        assert!(err.contains("Error running version script"), err);
        assert!(err.contains("some error"), err);
    }

    #[test]
    fn test_run_python_script() {
        let dir = TempDir::new("repo_version.rs").expect("create temp dir for repo_version.rs test");

        let sub_dir = dir.path().join("subdir");
        fs::create_dir(&sub_dir).expect("create subdir");

        let script_file = sub_dir.join("version.py");
        {
            let mut file = fs::File::create(&script_file).expect("create file");
            file.write_all(b"print '1.2.3.4'").expect("write file");
        }

        assert_eq!("1.2.3.4", run_script("python version.py", &sub_dir).unwrap());
    }

    #[test]
    fn test_run_script_isolation() {
        // firejail tmpfs isolation not quite working inside docker
        if docker::in_docker() {
            return;
        }

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
            file.write_all(
                br#"
            # no `set -e` on purpose: try to do them all!
            rm ../private.txt
            rm version.sh
            touch ../muahaha.txt
            touch muahaha.txt

            echo 1.2.3.4
            "#,
            )
            .expect("write file");
        }

        assert_eq!("1.2.3.4", run_script("bash version.sh", &sub_dir).unwrap());

        assert!(
            parent_file.exists(),
            "version scripts should not be able to delete files outside its directory"
        );
        assert!(
            !dir.path().join("muahaha.txt").exists(),
            "version scripts should not be able to create files outside its directory"
        );
    }
}
