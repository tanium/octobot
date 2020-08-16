use log;

use crate::errors::*;
use crate::jira;
use crate::github;

const JIRA_REF_CONTEXT: &'static str = "jira";

pub fn check_jira_refs(
    pull_request: &github::PullRequest,
    commits: &Vec<github::PushCommit>,
    projects: &Vec<String>,
    github: &dyn github::api::Session) {

    // Skip PRs named accordingl
    if pull_request.title.to_lowercase().starts_with("chore:") ||
       pull_request.title.to_lowercase().starts_with("build:") {
        return;
    }

    // Always skip projects with no JIRAs configured
    if projects.is_empty() {
        return;
    }

    if let Err(e) = do_check_jira_refs(pull_request, commits, projects, github) {
        log::error!("Error checking jira refs: {}", e);
    }
}

fn do_check_jira_refs(
    pull_request: &github::PullRequest,
    commits: &Vec<github::PushCommit>,
    projects: &Vec<String>,
    github: &dyn github::api::Session) -> Result<()> {

    let mut run = github::CheckRun::new(JIRA_REF_CONTEXT, pull_request, None);
    if jira::workflow::get_all_jira_keys(commits, projects).is_empty() {
        run = run.completed(github::Conclusion::Failure);

        let msg: String;
        if projects.len() == 1 {
            msg = format!("Expected a JIRA reference for the project: {}", projects[0]);
        } else {
            msg = format!("Expected a JIRA reference for at least one of the following projects: {}", projects.join(", "))
        }
        run.output = Some(github::CheckOutput::new("Missing JIRA reference", &msg));
    } else {
        run = run.completed(github::Conclusion::Success);
    }

    github.create_check_run(pull_request, &run)?;

    Ok(())
}
