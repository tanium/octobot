extern crate octobot;

mod mocks;

use mocks::mock_github::MockGithub;

use octobot::force_push;
use octobot::github;
use octobot::diffs::DiffOfDiffs;

#[test]
fn test_force_push_identical() {
    let mut pr = github::PullRequest::new();
    pr.number = 32;
    pr.head.ref_name = "the-pr-branch".into();

    let diff = "this is a big diff\n\nIt has lots of lines,\n\nbut it is the same".to_string();
    let diffs = Ok(DiffOfDiffs::new(&diff, &diff));

    let github = MockGithub::new();
    github.mock_comment_pull_request("some-user", "some-repo", 32,
        "Force-push detected: before: abcdef0, after: 1111abc: Identical diff post-rebase",
        Ok(()));

    github.mock_get_statuses("some-user", "some-repo", "abcdef0999999", Ok(vec![]));

    force_push::comment_force_push(diffs, vec![], &github, "some-user", "some-repo", &pr,
                                   "abcdef0999999", "1111abc9999999").unwrap();
}

#[test]
fn test_force_push_identical_with_statuses() {
    let mut pr = github::PullRequest::new();
    pr.number = 32;

    let diff = "this is a big diff\n\nIt has lots of lines,\n\nbut it is the same".to_string();
    let diffs = Ok(DiffOfDiffs::new(&diff, &diff));

    let github = MockGithub::new();
    github.mock_comment_pull_request("some-user", "some-repo", 32,
        "Force-push detected: before: the-bef, after: the-aft: Identical diff post-rebase",
        Ok(()));


    let statuses = vec![
        github::Status {
            state: "success".into(),
            target_url: Some("http://ci/build".into()),
            context: Some("ci/build".into()),
            description: Some("the desc".into()),
            creator: None,
        },
        github::Status {
            state: "failure".into(),
            target_url: None,
            context: Some("checks/cla".into()),
            description: None,
            creator: None,
        },
        github::Status {
            state: "error".into(),
            target_url: None,
            context: Some("checks/cla".into()), // duplicate context -- should be ignored
            description: None,
            creator: None,
        },
        github::Status {
            state: "pending".into(),
            target_url: None,
            context: Some("something/else".into()),
            description: None,
            creator: None,
        },
    ];

    github.mock_get_statuses("some-user", "some-repo", "the-before-hash", Ok(statuses));

    let new_status1 = github::Status {
        state: "success".into(),
        target_url: Some("http://ci/build".into()),
        context: Some("ci/build".into()),
        description: Some("the desc (reapplied by octobot)".into()),
        creator: None,
    };
    let new_status2 = github::Status {
        state: "failure".into(),
        target_url: None,
        context: Some("checks/cla".into()),
        description: Some("(reapplied by octobot)".into()),
        creator: None,
    };


    github.mock_create_status("some-user", "some-repo", "the-after-hash", &new_status1, Ok(()));
    github.mock_create_status("some-user", "some-repo", "the-after-hash", &new_status2, Ok(()));

    force_push::comment_force_push(diffs, vec!["ci/build".into(), "checks/cla".into()], &github,
                                   "some-user", "some-repo", &pr, "the-before-hash", "the-after-hash").unwrap();
}


#[test]
fn test_force_push_different() {
    let mut pr = github::PullRequest::new();
    pr.number = 32;

    let diffs = Ok(DiffOfDiffs::new("diff1", "diff2"));

    let github = MockGithub::new();
    github.mock_comment_pull_request("some-user", "some-repo", 32,
        "Force-push detected: before: abcdef0, after: 1111abc: Diff changed post-rebase",
        Ok(()));

    force_push::comment_force_push(diffs, vec![], &github, "some-user", "some-repo", &pr,
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

    force_push::comment_force_push(Err("Ahh!!".into()), vec![], &github, "some-user", "some-repo", &pr,
                                   "abcdef0999999", "1111abc9999999").unwrap();
}
