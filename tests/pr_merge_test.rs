extern crate octobot;
extern crate tempdir;

mod git_helper;
mod mocks;

use git_helper::temp_git::TempGit;
use mocks::mock_github::MockGithub;
use octobot::github;
use octobot::pr_merge;

#[test]
fn test_pr_merge_basic() {
    let git = TempGit::new();
    let github = MockGithub::new();

    // setup a release branch
    git.run_git(&["push", "origin", "master:release/1.0"]);

    // make a new commit on master
    git.run_git(&["checkout", "master"]);
    git.add_repo_file("file.txt", "contents1", "I made a change");
    let commit1 = git.git.current_commit().unwrap();

    // pretend this came from a PR
    let mut pr = github::PullRequest::new();
    pr.number = 123;
    pr.merged = Some(true);
    pr.merge_commit_sha = Some(commit1.clone());
    pr.head = github::BranchRef::new("my-feature-branch");
    pr.base = github::BranchRef::new("master");
    pr.assignees = vec![github::User::new("user1"), github::User::new("user2"), github::User::new("the-pr-author")];
    pr.requested_reviewers = Some(vec![github::User::new("reviewer1")]);
    pr.reviews = Some(vec![
        github::Review::new("fantastic change", github::User::new("reviewer2")),
        github::Review::new("i like to comment on my own PRs", github::User::new("the-pr-author")),
    ]);
    pr.user = github::User::new("the-pr-author");
    let pr = pr;

    let mut new_pr = github::PullRequest::new();
    new_pr.number = 456;
    let new_pr = new_pr;

    github.mock_create_pull_request(
        "the-owner",
        "the-repo",
        "master->1.0: I made a change",
        &format!("(cherry-picked from {}, PR #123)", commit1),
        "my-feature-branch-1.0",
        "release/1.0",
        Ok(new_pr),
    );

    github.mock_assign_pull_request(
        "the-owner",
        "the-repo",
        456,
        vec!["user1".into(), "user2".into()],
        Ok(()),
    );

    github.mock_request_review(
        "the-owner",
        "the-repo",
        456,
        vec!["reviewer1".into(), "reviewer2".into()],
        Ok(()),
    );

    let created_pr = pr_merge::merge_pull_request(&git.git, &github, "the-owner", "the-repo", &pr, "release/1.0")
        .unwrap();

    assert_eq!(456, created_pr.number);
    assert_eq!("", git.run_git(&["diff", "master", "origin/my-feature-branch-1.0"]));
}

#[test]
fn test_pr_merge_ignore_space_change() {
    let git = TempGit::new();
    let github = MockGithub::new();

    let contents_base = "
if (true) {
    do_something();
}
";
    let contents_master = "
try {
    if (true) {
        do_something();
    }
} catch ( ... ) {
    aha();
}
";
    // note the extra space before the brace...
    let contents_10 = "
if (true)    {
    do_something();
}
";
    let contents_10_final = "try {
    if (true) {
        do_something();
    }
} catch ( ... ) {
    aha();
}";

    // base contents
    git.add_repo_file("file.cpp", contents_base, "base");

    // setup a release branch: make change in space length
    git.run_git(&["checkout", "-b", "release/1.0"]);
    git.add_repo_file("file.cpp", contents_10, "a change on 1.0");
    git.run_git(&["push", "-u", "origin", "release/1.0"]);

    // make a new commit on master
    git.run_git(&["checkout", "master"]);
    git.add_repo_file("file.cpp", contents_master, "final change");
    let commit1 = git.git.current_commit().unwrap();

    // pretend this came from a PR
    let mut pr = github::PullRequest::new();
    pr.number = 123;
    pr.merged = Some(true);
    pr.merge_commit_sha = Some(commit1.clone());
    pr.head = github::BranchRef::new("my-feature-branch");
    pr.base = github::BranchRef::new("master");
    pr.assignees = vec![github::User::new("user1"), github::User::new("user2")];
    let pr = pr;

    let mut new_pr = github::PullRequest::new();
    new_pr.number = 456;
    let new_pr = new_pr;

    github.mock_create_pull_request(
        "the-owner",
        "the-repo",
        "master->1.0: final change",
        &format!("(cherry-picked from {}, PR #123)", commit1),
        "my-feature-branch-1.0",
        "release/1.0",
        Ok(new_pr),
    );

    github.mock_assign_pull_request(
        "the-owner",
        "the-repo",
        456,
        vec!["user1".into(), "user2".into()],
        Ok(()),
    );

    github.mock_comment_pull_request(
        "the-owner",
        "the-repo",
        456,
        "Cherry-pick required option `ignore-space-change`. Please verify correctness.",
        Ok(()),
    );

    let created_pr = pr_merge::merge_pull_request(&git.git, &github, "the-owner", "the-repo", &pr, "release/1.0")
        .unwrap();

    assert_eq!(456, created_pr.number);
    assert_eq!(contents_10_final, git.run_git(&["cat-file", "blob", "my-feature-branch-1.0:file.cpp"]));
}

#[test]
fn test_pr_merge_ignore_all_space() {
    let git = TempGit::new();
    let github = MockGithub::new();

    let contents_base = "
if (true) {
    do_something();
}
";
    let contents_master = "
try {
    if (true) {
        do_something();
    }
} catch ( ... ) {
    aha();
}
";
    let contents_10 = "
if (true) {
    do_something_else();
}
";
    let contents_10_final = "try {
if (true) {
    do_something_else();
}
} catch ( ... ) {
    aha();
}";

    // base contents
    git.add_repo_file("file.cpp", contents_base, "base");

    // setup a release branch: make a change
    git.run_git(&["checkout", "-b", "release/1.0"]);
    git.add_repo_file("file.cpp", contents_10, "a change on 1.0");
    git.run_git(&["push", "-u", "origin", "release/1.0"]);

    // make a new commit on master
    git.run_git(&["checkout", "master"]);
    git.add_repo_file("file.cpp", contents_master, "final change");
    let commit1 = git.git.current_commit().unwrap();

    // pretend this came from a PR
    let mut pr = github::PullRequest::new();
    pr.number = 123;
    pr.merged = Some(true);
    pr.merge_commit_sha = Some(commit1.clone());
    pr.head = github::BranchRef::new("my-feature-branch");
    pr.base = github::BranchRef::new("master");
    pr.assignees = vec![github::User::new("user1"), github::User::new("user2")];
    let pr = pr;

    let mut new_pr = github::PullRequest::new();
    new_pr.number = 456;
    let new_pr = new_pr;

    github.mock_create_pull_request(
        "the-owner",
        "the-repo",
        "master->1.0: final change",
        &format!("(cherry-picked from {}, PR #123)", commit1),
        "my-feature-branch-1.0",
        "release/1.0",
        Ok(new_pr),
    );

    github.mock_assign_pull_request(
        "the-owner",
        "the-repo",
        456,
        vec!["user1".into(), "user2".into()],
        Ok(()),
    );

    github.mock_comment_pull_request(
        "the-owner",
        "the-repo",
        456,
        "Cherry-pick required option `ignore-all-space`. Please verify correctness.",
        Ok(()),
    );

    let created_pr = pr_merge::merge_pull_request(&git.git, &github, "the-owner", "the-repo", &pr, "release/1.0")
        .unwrap();

    assert_eq!(456, created_pr.number);
    assert_eq!(contents_10_final, git.run_git(&["cat-file", "blob", "my-feature-branch-1.0:file.cpp"]));
}
