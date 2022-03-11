mod mocks;

use mocks::mock_github::MockGithub;

use octobot_lib::github;
use octobot_lib::jira;

fn new_pr(title: &str) -> github::PullRequest {
    let mut pr = github::PullRequest::new();
    pr.number = 1;
    pr.title = title.into();
    pr
}

fn new_commit(msg: &str) -> github::Commit {
    let mut commit = github::Commit::new();
    commit.commit.message = msg.into();
    commit
}

fn expect_pass(git: &MockGithub, pr: &github::PullRequest) {
    expect_pass_commit(git, pr, &pr.head.sha);
}

fn expect_pass_commit(git: &MockGithub, pr: &github::PullRequest, commit: &str) {
    git.mock_create_check_run(
        pr,
        &github::CheckRun::new("jira", commit, None).completed(github::Conclusion::Success),
        Ok(1),
    );
}

fn expect_failure(git: &MockGithub, pr: &github::PullRequest) {
    let mut run =
        github::CheckRun::new("jira", &pr.head.sha, None).completed(github::Conclusion::Neutral);
    run.output = Some(github::CheckOutput::new("Missing JIRA reference", ""));

    git.mock_create_check_run(pr, &run, Ok(1));
}

fn expect_skip(git: &MockGithub, pr: &github::PullRequest) {
    let mut run =
        github::CheckRun::new("jira", &pr.head.sha, None).completed(github::Conclusion::Neutral);
    run.output = Some(github::CheckOutput::new("Skipped JIRA check", ""));

    git.mock_create_check_run(pr, &run, Ok(1));
}

#[tokio::test]
async fn test_check_jira_refs_no_projects() {
    let git = MockGithub::new();

    let pr = new_pr("");
    let commits = vec![new_commit("did stuff")];
    let projects = vec![];

    // No assertions -- it shouldn't do anything

    jira::check_jira_refs(&pr, &commits, &projects, &git).await;
}

#[tokio::test]
async fn test_check_jira_refs_chore_commit() {
    let git = MockGithub::new();

    let pr = new_pr("chore: Do stuff");
    let commits = vec![new_commit("did stuff")];
    let projects = vec!["SERVER".into()];

    expect_skip(&git, &pr);

    jira::check_jira_refs(&pr, &commits, &projects, &git).await;
}

#[tokio::test]
async fn test_check_jira_refs_chore_commit_scope() {
    let git = MockGithub::new();

    let pr = new_pr("chore(deps): Update deps");
    let commits = vec![new_commit("update deps")];
    let projects = vec!["SERVER".into()];

    expect_skip(&git, &pr);

    jira::check_jira_refs(&pr, &commits, &projects, &git).await;
}

#[tokio::test]
async fn test_check_jira_refs_build_commit() {
    let git = MockGithub::new();

    let pr = new_pr("build: do stuff");
    let commits = vec![new_commit("did stuff")];
    let projects = vec!["SERVER".into()];

    expect_skip(&git, &pr);

    jira::check_jira_refs(&pr, &commits, &projects, &git).await;
}

#[tokio::test]
async fn test_check_jira_refs_mismatch() {
    let git = MockGithub::new();

    let pr = new_pr("Do stuff");
    let commits = vec![new_commit("[SERVER-123] Do stuff")];
    let projects = vec!["CLIENT".into()];

    expect_failure(&git, &pr);

    jira::check_jira_refs(&pr, &commits, &projects, &git).await;
}

#[tokio::test]
async fn test_check_jira_refs_pass() {
    let git = MockGithub::new();

    let pr = new_pr("Do stuff");
    let commits = vec![new_commit("[SERVER-123] Do stuff")];
    let projects = vec!["SERVER".into()];

    expect_pass(&git, &pr);

    jira::check_jira_refs(&pr, &commits, &projects, &git).await;
}

#[tokio::test]
async fn test_check_jira_refs_requires_only_one_ref() {
    let git = MockGithub::new();

    let pr = new_pr("Do stuff");
    let commits = vec![new_commit("[SERVER-123] Do stuff")];
    let projects = vec!["SERVER".into(), "CLIENT".into()];

    expect_pass(&git, &pr);

    jira::check_jira_refs(&pr, &commits, &projects, &git).await;
}

#[tokio::test]
async fn test_check_jira_refs_checks_all_commits() {
    let git = MockGithub::new();

    let pr = new_pr("Do stuff");
    let commits = vec![
        new_commit("Do stuff"),
        new_commit("Fix [CLIENT-123] whoops, add jira ref"),
    ];
    let projects = vec!["SERVER".into(), "CLIENT".into()];

    expect_pass(&git, &pr);

    jira::check_jira_refs(&pr, &commits, &projects, &git).await;
}

#[tokio::test]
async fn test_check_jira_refs_pr_head_mismatch() {
    let git = MockGithub::new();

    let mut pr = new_pr("Do stuff");
    let mut commits = vec![
        new_commit("Do stuff"),
        new_commit("Fix [CLIENT-123] whoops, add jira ref"),
    ];
    let projects = vec!["SERVER".into(), "CLIENT".into()];

    pr.head.sha = "1".to_string();
    commits[0].sha = "1".to_string();
    commits[1].sha = "2".to_string();

    expect_pass_commit(&git, &pr, &commits[1].sha);

    jira::check_jira_refs(&pr, &commits, &projects, &git).await;
}
