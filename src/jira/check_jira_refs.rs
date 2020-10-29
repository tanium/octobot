use log;
use conventional::{Commit, Simple as _};

use crate::errors::*;
use crate::jira;
use crate::github;

const JIRA_REF_CONTEXT: &'static str = "jira";

const ALLOWED_SKIP_TYPES : &'static [&'static str] = &[
    "chore",
    "build",
    "refactor",
    "style",
    "test",
];

pub fn check_jira_refs(
    pull_request: &github::PullRequest,
    commits: &Vec<github::Commit>,
    projects: &Vec<String>,
    github: &dyn github::api::Session) {

    // Always skip projects with no JIRAs configured
    if projects.is_empty() {
        return;
    }

    // Skip PRs titled accordingly.
    if let Some(commit_type) = conventional_commit_jira_skip_type(&pull_request.title) {
        if let Err(e) = do_skip_jira_check(pull_request, commit_type, github) {
            log::error!("Error marking skipped jira refs: {}", e);
        }
        return;
    }

    if let Err(e) = do_check_jira_refs(pull_request, commits, projects, github) {
        log::error!("Error checking jira refs: {}", e);
    }
}

// Note: this requires PR commits, not push commits, because we want to take all PR commits into
// consideration, not just what was recently pushed.
fn do_check_jira_refs(
    pull_request: &github::PullRequest,
    commits: &Vec<github::Commit>,
    projects: &Vec<String>,
    github: &dyn github::api::Session) -> Result<()> {

    let mut run = github::CheckRun::new(JIRA_REF_CONTEXT, pull_request, None);
    if jira::workflow::get_all_jira_keys(commits, projects).is_empty() {
        run = run.completed(github::Conclusion::Neutral);

        let msg: String;
        if projects.len() == 1 {
            msg = format!("Expected a JIRA reference in a commit message for the project {}", projects[0]);
        } else {
            msg = format!("Expected a JIRA reference in a commit message for at least one of the following projects: {}", projects.join(", "))
        }
        run.output = Some(github::CheckOutput::new("Missing JIRA reference", &msg));
    } else {
        run = run.completed(github::Conclusion::Success);
    }

    github.create_check_run(pull_request, &run)?;

    Ok(())
}

fn conventional_commit_jira_skip_type(title: &str) -> Option<&str> {
    match Commit::new(title) {
        Ok(commit) => {
            for t in ALLOWED_SKIP_TYPES {
                if *t == commit.type_() {
                    return Some(t)
                }
            }

            return None;
        }
        Err(_) => {
            // no conventional commit: require jira
            None
        },
    }
}

fn do_skip_jira_check(
    pull_request: &github::PullRequest,
    commit_type: &str,
    github: &dyn github::api::Session) -> Result<()> {

    let msg = "Skipped JIRA check";
    let body = format!("Skipped JIRA check for commit type: {}", commit_type);

    let mut run = github::CheckRun::new(JIRA_REF_CONTEXT, pull_request, None);
    run = run.completed(github::Conclusion::Neutral);
    run.output = Some(github::CheckOutput::new(&msg, &body));

    github.create_check_run(pull_request, &run)?;

    Ok(())
}
