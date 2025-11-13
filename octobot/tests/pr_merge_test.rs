mod git_helper;
mod mocks;

use std::sync::Arc;

use tempfile::{TempDir, tempdir};

use git_helper::temp_git::TempGit;
use mocks::mock_github::MockGithub;
use octobot_lib::config::Config;
use octobot_lib::config_db::ConfigDatabase;
use octobot_lib::github;
use octobot_lib::repos;
use octobot_lib::slack::SlackRecipient;
use octobot_ops::pr_merge;
use octobot_ops::slack::{self, SlackAttachmentBuilder};

use anyhow::anyhow;

use mocks::mock_slack::MockSlack;

struct PRMergeTest {
    git: TempGit,
    github: MockGithub,
    config: Arc<Config>,
    slack: MockSlack,
}

fn new_test() -> (PRMergeTest, TempDir) {
    let temp_dir = tempdir().unwrap();
    let db_file = temp_dir.path().join("db.sqlite3");
    let db = ConfigDatabase::new(&db_file.to_string_lossy()).expect("create temp database");

    let config = Arc::new(Config::new(db));
    config
        .users_write()
        .insert("the-pr-owner", "the.pr.owner")
        .unwrap();
    config
        .repos_write()
        .insert_info(
            &repos::RepoInfo::new("the-owner/the-repo", "the-review-channel")
                .with_jira("SER")
                .with_jira("CLI")
                .with_force_push(true),
        )
        .expect("Failed to add some-user/some-repo");

    (
        PRMergeTest {
            git: TempGit::new(),
            github: MockGithub::new(),
            config,
            slack: MockSlack::new(vec![]),
        },
        temp_dir,
    )
}

#[tokio::test]
async fn test_pr_merge_basic() {
    let (test, _temp_dir) = new_test();

    // setup a release branch
    test.git.run_git(&["push", "origin", "master:release/1.0"]);

    // make a new commit on master
    test.git.run_git(&["checkout", "master"]);
    test.git
        .add_repo_file("file.txt", "contents1", "I made a change");
    let commit1 = test.git.git.current_commit().unwrap();

    // pretend this came from a PR
    let mut pr = github::PullRequest::new();
    pr.number = 123;
    pr.merged = Some(true);
    pr.merge_commit_sha = Some(commit1.clone());
    pr.head = github::BranchRef::new("my-feature-branch");
    pr.base = github::BranchRef::new("master");
    pr.assignees = vec![
        github::User::new("user1"),
        github::User::new("user2"),
        github::User::new("the-pr-author"),
    ];
    pr.requested_reviewers = Some(vec![github::User::new("reviewer1")]);
    pr.reviews = Some(vec![
        github::Review::new("fantastic change", github::User::new("reviewer2")),
        github::Review::new(
            "i like to comment on my own PRs",
            github::User::new("the-pr-author"),
        ),
    ]);
    pr.user = github::User::new("the-pr-author");
    let pr = pr;

    let mut new_pr = github::PullRequest::new();
    new_pr.number = 456;
    let new_pr = new_pr;

    test.github.mock_create_pull_request(
        "the-owner",
        "the-repo",
        "master->1.0: I made a change",
        &format!("(cherry-picked from {}, PR #123)", commit1),
        "my-feature-branch-1.0",
        "release/1.0",
        Ok(new_pr),
    );

    test.github.mock_assign_pull_request(
        "the-owner",
        "the-repo",
        456,
        vec!["user1".into(), "user2".into(), "the-pr-author".into()],
        Ok(()),
    );

    test.github.mock_request_review(
        "the-owner",
        "the-repo",
        456,
        vec!["reviewer1".into(), "reviewer2".into()],
        Ok(()),
    );

    let repo = github::Repo::parse("http://the-github-host/the-owner/the-repo").unwrap();
    let req = pr_merge::req(&repo, &pr, "release/1.0", "release/", &[]);
    pr_merge::merge_pull_request(
        &test.git.git,
        &test.github,
        &req,
        test.config,
        test.slack.new_sender(),
    )
    .await;

    let (user, email) = test
        .git
        .git
        .get_commit_author("origin/my-feature-branch-1.0")
        .unwrap();

    assert_eq!(user, test.git.user_name());
    assert_eq!(email, test.git.user_email());

    assert_eq!(
        "",
        test.git
            .run_git(&["diff", "master", "origin/my-feature-branch-1.0"])
    );
}

#[tokio::test]
async fn test_pr_merge_author_is_assignee() {
    let (test, _temp_dir) = new_test();

    // setup a release branch
    test.git.run_git(&["push", "origin", "master:release/1.0"]);

    // make a new commit on master
    test.git.run_git(&["checkout", "master"]);
    test.git
        .add_repo_file("file.txt", "contents1", "I made a change");
    let commit1 = test.git.git.current_commit().unwrap();

    // pretend this came from a PR
    let mut pr = github::PullRequest::new();
    pr.number = 123;
    pr.merged = Some(true);
    pr.merge_commit_sha = Some(commit1.clone());
    pr.head = github::BranchRef::new("my-feature-branch");
    pr.base = github::BranchRef::new("master");
    pr.user = github::User::new("the-pr-author");
    let pr = pr;

    let mut new_pr = github::PullRequest::new();
    new_pr.number = 456;
    let new_pr = new_pr;

    test.github.mock_create_pull_request(
        "the-owner",
        "the-repo",
        "master->1.0: I made a change",
        &format!("(cherry-picked from {}, PR #123)", commit1),
        "my-feature-branch-1.0",
        "release/1.0",
        Ok(new_pr),
    );

    test.github.mock_assign_pull_request(
        "the-owner",
        "the-repo",
        456,
        vec!["the-pr-author".into()],
        Ok(()),
    );

    let repo = github::Repo::parse("http://the-github-host/the-owner/the-repo").unwrap();
    let req = pr_merge::req(&repo, &pr, "release/1.0", "release/", &[]);
    pr_merge::merge_pull_request(
        &test.git.git,
        &test.github,
        &req,
        test.config,
        test.slack.new_sender(),
    )
    .await;
}

#[tokio::test]
async fn test_pr_merge_ignore_space_change() {
    let (test, _temp_dir) = new_test();

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
    test.git.add_repo_file("file.cpp", contents_base, "base");

    // setup a release branch: make change in space length
    test.git.run_git(&["checkout", "-b", "release/1.0"]);
    test.git
        .add_repo_file("file.cpp", contents_10, "a change on 1.0");
    test.git.run_git(&["push", "-u", "origin", "release/1.0"]);

    // make a new commit on master
    test.git.run_git(&["checkout", "master"]);
    test.git
        .add_repo_file("file.cpp", contents_master, "final change");
    let commit1 = test.git.git.current_commit().unwrap();

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

    test.github.mock_create_pull_request(
        "the-owner",
        "the-repo",
        "master->1.0: final change",
        &format!("(cherry-picked from {}, PR #123)", commit1),
        "my-feature-branch-1.0",
        "release/1.0",
        Ok(new_pr),
    );

    test.github.mock_assign_pull_request(
        "the-owner",
        "the-repo",
        456,
        vec!["user1".into(), "user2".into()],
        Ok(()),
    );

    test.github.mock_comment_pull_request(
        "the-owner",
        "the-repo",
        456,
        "Cherry-pick required option `ignore-space-change`. Please verify correctness.",
        Ok(()),
    );

    let repo = github::Repo::parse("http://the-github-host/the-owner/the-repo").unwrap();
    let req = pr_merge::req(&repo, &pr, "release/1.0", "release/", &[]);
    pr_merge::merge_pull_request(
        &test.git.git,
        &test.github,
        &req,
        test.config,
        test.slack.new_sender(),
    )
    .await;

    assert_eq!(
        contents_10_final,
        test.git
            .run_git(&["cat-file", "blob", "my-feature-branch-1.0:file.cpp"])
    );
}

#[tokio::test]
async fn test_pr_merge_ignore_all_space() {
    let (test, _temp_dir) = new_test();

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
    test.git.add_repo_file("file.cpp", contents_base, "base");

    // setup a release branch: make a change
    test.git.run_git(&["checkout", "-b", "release/1.0"]);
    test.git
        .add_repo_file("file.cpp", contents_10, "a change on 1.0");
    test.git.run_git(&["push", "-u", "origin", "release/1.0"]);

    // make a new commit on master
    test.git.run_git(&["checkout", "master"]);
    test.git
        .add_repo_file("file.cpp", contents_master, "final change");
    let commit1 = test.git.git.current_commit().unwrap();

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

    test.github.mock_create_pull_request(
        "the-owner",
        "the-repo",
        "master->1.0: final change",
        &format!("(cherry-picked from {}, PR #123)", commit1),
        "my-feature-branch-1.0",
        "release/1.0",
        Ok(new_pr),
    );

    test.github.mock_assign_pull_request(
        "the-owner",
        "the-repo",
        456,
        vec!["user1".into(), "user2".into()],
        Ok(()),
    );

    test.github.mock_comment_pull_request(
        "the-owner",
        "the-repo",
        456,
        "Cherry-pick required option `ignore-all-space`. Please verify correctness.",
        Ok(()),
    );

    let repo = github::Repo::parse("http://the-github-host/the-owner/the-repo").unwrap();
    let req = pr_merge::req(&repo, &pr, "release/1.0", "release/", &[]);
    pr_merge::merge_pull_request(
        &test.git.git,
        &test.github,
        &req,
        test.config,
        test.slack.new_sender(),
    )
    .await;

    assert_eq!(
        contents_10_final,
        test.git
            .run_git(&["cat-file", "blob", "my-feature-branch-1.0:file.cpp"])
    );
}

#[tokio::test]
async fn test_pr_merge_conventional_commit() {
    let (test, _temp_dir) = new_test();

    // setup a release branch
    test.git.run_git(&["push", "origin", "master:release/1.0"]);

    // make a new commit on master
    test.git.run_git(&["checkout", "master"]);
    test.git
        .add_repo_file("file.txt", "contents1", "fix(thing)!: I made a change");
    let commit1 = test.git.git.current_commit().unwrap();

    // pretend this came from a PR
    let mut pr = github::PullRequest::new();
    pr.number = 123;
    pr.merged = Some(true);
    pr.merge_commit_sha = Some(commit1.clone());
    pr.head = github::BranchRef::new("my-feature-branch");
    pr.base = github::BranchRef::new("master");
    pr.assignees = vec![
        github::User::new("user1"),
        github::User::new("user2"),
        github::User::new("the-pr-author"),
    ];
    pr.requested_reviewers = Some(vec![github::User::new("reviewer1")]);
    pr.reviews = Some(vec![
        github::Review::new("fantastic change", github::User::new("reviewer2")),
        github::Review::new(
            "i like to comment on my own PRs",
            github::User::new("the-pr-author"),
        ),
    ]);
    pr.user = github::User::new("the-pr-author");
    let pr = pr;

    let mut new_pr = github::PullRequest::new();
    new_pr.number = 456;
    let new_pr = new_pr;

    test.github.mock_create_pull_request(
        "the-owner",
        "the-repo",
        "fix(thing)!: master->1.0: I made a change",
        &format!("(cherry-picked from {}, PR #123)", commit1),
        "my-feature-branch-1.0",
        "release/1.0",
        Ok(new_pr),
    );

    test.github.mock_assign_pull_request(
        "the-owner",
        "the-repo",
        456,
        vec!["user1".into(), "user2".into(), "the-pr-author".into()],
        Ok(()),
    );

    test.github.mock_request_review(
        "the-owner",
        "the-repo",
        456,
        vec!["reviewer1".into(), "reviewer2".into()],
        Ok(()),
    );

    let repo = github::Repo::parse("http://the-github-host/the-owner/the-repo").unwrap();
    let req = pr_merge::req(&repo, &pr, "release/1.0", "release/", &[]);
    pr_merge::merge_pull_request(
        &test.git.git,
        &test.github,
        &req,
        test.config,
        test.slack.new_sender(),
    )
    .await;

    let (user, email) = test
        .git
        .git
        .get_commit_author("origin/my-feature-branch-1.0")
        .unwrap();

    assert_eq!(user, test.git.user_name());
    assert_eq!(email, test.git.user_email());

    assert_eq!(
        "",
        test.git
            .run_git(&["diff", "master", "origin/my-feature-branch-1.0"])
    );
}

#[tokio::test]
async fn test_pr_merge_backport_failure() {
    let (mut test, _temp_dir) = new_test();

    // setup a release branch
    test.git.run_git(&["push", "origin", "master:release/1.0"]);

    // make a new commit on master
    test.git.run_git(&["checkout", "master"]);
    test.git
        .add_repo_file("file.txt", "contents1", "I made a change");
    let commit1 = test.git.git.current_commit().unwrap();

    // pretend this came from a PR
    let mut pr = github::PullRequest::new();
    pr.number = 123;
    pr.title = "The Title".into();
    pr.merged = Some(true);
    pr.merge_commit_sha = Some(commit1.clone());
    pr.head = github::BranchRef::new("my-feature-branch");
    pr.base = github::BranchRef::new("master");
    pr.assignees = vec![
        github::User::new("user1"),
        github::User::new("user2"),
        github::User::new("the-pr-author"),
    ];
    pr.requested_reviewers = Some(vec![github::User::new("reviewer1")]);
    pr.reviews = Some(vec![
        github::Review::new("fantastic change", github::User::new("reviewer2")),
        github::Review::new(
            "i like to comment on my own PRs",
            github::User::new("the-pr-author"),
        ),
    ]);
    pr.user = github::User::new("the-pr-author");
    let pr = pr;

    test.github.mock_create_pull_request(
        "the-owner",
        "the-repo",
        "master->1.0: I made a change",
        &format!("(cherry-picked from {}, PR #123)", commit1),
        "my-feature-branch-1.0",
        "release/1.0",
        Err(anyhow!("bad stuff")),
    );

    test.github.mock_comment_pull_request(
        "the-owner",
        "the-repo",
        123,
        "Error backporting PR from my-feature-branch to release/1.0\n<details>\n<summary>Details</summary>\n\n```\nbad stuff\n```\n</details>",
        Ok(()),
    );

    test.github.mock_add_pull_request_labels(
        "the-owner",
        "the-repo",
        123,
        vec!["failed-backport".to_string()],
        Ok(()),
    );

    test.slack.expect(vec![
        slack::req(
            SlackRecipient::by_name("the-review-channel"),
            "Error backporting PR from my-feature-branch to release/1.0 (<http://the-github-host/the-owner/the-repo|the-owner/the-repo>)",
            &[SlackAttachmentBuilder::new("")
                .markdown("Error backporting PR from my-feature-branch to release/1.0\n\n```\nbad stuff\n```")
                .title("Source PR: #123: \"The Title\"")
                .title_link("")
                .color("danger")
                .build()],
            None,
            false,
        )
    ]);

    let repo = github::Repo::parse("http://the-github-host/the-owner/the-repo").unwrap();
    let req = pr_merge::req(&repo, &pr, "release/1.0", "release/", &[]);
    pr_merge::merge_pull_request(
        &test.git.git,
        &test.github,
        &req,
        test.config,
        test.slack.new_sender(),
    )
    .await;
}
