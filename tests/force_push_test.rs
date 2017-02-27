extern crate octobot;

mod mocks;

use mocks::mock_github::MockGithub;

use octobot::force_push;
use octobot::github;

#[test]
fn test_force_push_identical() {
    let mut pr = github::PullRequest::new();
    pr.number = 32;
    pr.head.ref_name = "the-pr-branch".into();

    let diff = "this is a big diff\n\nIt has lots of lines,\n\nbut it is the same".to_string();
    let diffs = Ok((diff.clone(), diff.clone()));

    let github = MockGithub::new();
    github.mock_comment_pull_request("some-user", "some-repo", 32,
        "Force-push detected: before: abcdef0, after: 1111abc: Identical diff post-rebase",
        Ok(()));

    force_push::comment_force_push(diffs, &github, "some-user", "some-repo", &pr,
                                   "abcdef0999999", "1111abc9999999").unwrap();
}

#[test]
fn test_force_push_different() {
    let mut pr = github::PullRequest::new();
    pr.number = 32;

    let diffs = Ok(("diff1".into(), "diff2".into()));

    let github = MockGithub::new();
    github.mock_comment_pull_request("some-user", "some-repo", 32,
        "Force-push detected: before: abcdef0, after: 1111abc: Diff changed post-rebase",
        Ok(()));

    force_push::comment_force_push(diffs, &github, "some-user", "some-repo", &pr,
                                   "abcdef0999999", "1111abc9999999").unwrap();
}

#[test]
fn test_force_push_error() {
    let mut pr = github::PullRequest::new();
    pr.number = 32;

    let github = MockGithub::new();
    github.mock_comment_pull_request("some-user", "some-repo", 32,
        "Force-push detected: before: abcdef0, after: 1111abc: Unable to calculate diff",
        Ok(()));

    force_push::comment_force_push(Err("Ahh!!".into()), &github, "some-user", "some-repo", &pr,
                                   "abcdef0999999", "1111abc9999999").unwrap();
}
