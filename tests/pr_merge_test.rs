extern crate octobot;
extern crate tempdir;

mod git_helper;
mod mocks;

use git_helper::temp_git::TempGit;
use mocks::mock_github::MockGithub;
use octobot::github;
use octobot::pr_merge;

#[test]
fn test_pr_merge() {
    let git = TempGit::new();
    let github = MockGithub::new();

    // setup a release branch
    git.run_git(&["push", "origin", "master:release/1.0"]);

    // make a new commit on master
    git.run_git(&["checkout", "master"]);
    git.add_repo_file("file1.txt", "contents1", "I made a change");
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
        Ok(github::AssignResponse { assignees: vec![] }),
    );

    let created_pr = pr_merge::merge_pull_request(&git.git, &github, "the-owner", "the-repo", &pr, "release/1.0")
        .unwrap();

    assert_eq!(456, created_pr.number);
    assert_eq!("", git.run_git(&["diff", "master", "origin/my-feature-branch-1.0"]));
}
