extern crate iron;
extern crate octobot;

mod mocks;

use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc;

use iron::status;

use octobot::config::Config;
use octobot::repos::RepoConfig;
use octobot::users::UserConfig;
use octobot::github::*;
use octobot::github::api::Session;
use octobot::messenger::SlackMessenger;
use octobot::slack::SlackAttachmentBuilder;
use octobot::server::github_handler::GithubEventHandler;

use mocks::mock_github::MockGithub;
use mocks::mock_slack::{SlackCall, MockSlack};

// this message gets appended only to review channel messages, not to slackbots
const REPO_MSG : &'static str = "(<http://the-github-host/some-user/some-repo|some-user/some-repo>)";

fn the_repo() -> Repo {
    Repo::parse("http://the-github-host/some-user/some-repo").unwrap()
}

fn new_messenger(slack: MockSlack, config: Arc<Config>) -> SlackMessenger {
    SlackMessenger {
        config: config,
        slack: Rc::new(slack),
    }
}

fn new_handler() -> GithubEventHandler {
    let github = MockGithub::new();
    let (tx, _) = mpsc::channel();
    let slack = MockSlack::new(vec![]);

    let mut repos = RepoConfig::new();
    let mut data = HookBody::new();

    repos.insert(github.github_host(),
                 "some-user/some-repo",
                 "the-reviews-channel");
    data.repository = Repo::parse(&format!("http://{}/some-user/some-repo", github.github_host()))
        .unwrap();
    data.sender = User::new("joe-sender");

    let config = Arc::new(Config::new(UserConfig::new(), repos));

    GithubEventHandler {
        event: "ping".to_string(),
        data: data,
        action: "".to_string(),
        config: config.clone(),
        messenger: Box::new(new_messenger(slack, config.clone())),
        github_session: Arc::new(github),
        pr_merge: tx.clone(),
    }
}

fn some_pr() -> Option<PullRequest> {
    Some(PullRequest {
        title: "The PR".into(),
        number: 32,
        html_url: "http://the-pr".into(),
        state: "open".into(),
        user: User::new("the-pr-owner"),
        merged: None,
        merge_commit_sha: None,
        assignees: vec![User::new("assign1"), User::new("joe-reviewer")],
        head: BranchRef {
            ref_name: "pr-branch".into(),
            sha: "ffff0000".into(),
            user: User::new("some-user"),
            repo: the_repo(),
        },
        base: BranchRef {
            ref_name: "master".into(),
            sha: "1111eeee".into(),
            user: User::new("some-user"),
            repo: the_repo(),
        },
    })
}

#[test]
fn test_ping() {
    let mut handler = new_handler();
    handler.event = "ping".to_string();

    let resp = handler.handle_event().unwrap();
    assert_eq!((status::Ok, "ping".into()), resp);
}

#[test]
fn test_commit_comment_with_path() {
    let mut handler = new_handler();
    handler.event = "commit_comment".into();
    handler.action = "created".into();
    handler.data.comment = Some(Comment {
        commit_id: Some("abcdef00001111".into()),
        path: Some("src/main.rs".into()),
        body: Some("I think this file should change".into()),
        html_url: "http://the-comment".into(),
        user: User::new("joe-reviewer"),
    });
    handler.data.sender = User::new("joe-reviewer");

    handler.messenger = Box::new(new_messenger(
        MockSlack::new(vec![
            SlackCall::new(
                "the-reviews-channel",
                &format!("Comment on \"src/main.rs\" (<http://the-github-host/some-user/some-repo/commit/abcdef00001111|abcdef0>) {}", REPO_MSG),
                vec![SlackAttachmentBuilder::new("I think this file should change")
                    .title("joe.reviewer said:")
                    .title_link("http://the-comment")
                    .build()]
            )
        ]),
        handler.config.clone()
    ));

    let resp = handler.handle_event().unwrap();
    assert_eq!((status::Ok, "commit_comment".into()), resp);
}

#[test]
fn test_commit_comment_no_path() {
    let mut handler = new_handler();
    handler.event = "commit_comment".into();
    handler.action = "created".into();
    handler.data.comment = Some(Comment {
        commit_id: Some("abcdef00001111".into()),
        path: None,
        body: Some("I think this file should change".into()),
        html_url: "http://the-comment".into(),
        user: User::new("joe-reviewer"),
    });
    handler.data.sender = User::new("joe-reviewer");

    handler.messenger = Box::new(new_messenger(
        MockSlack::new(vec![
            SlackCall::new(
                "the-reviews-channel",
                &format!("Comment on \"abcdef0\" (<http://the-github-host/some-user/some-repo/commit/abcdef00001111|abcdef0>) {}", REPO_MSG),
                vec![SlackAttachmentBuilder::new("I think this file should change")
                    .title("joe.reviewer said:")
                    .title_link("http://the-comment")
                    .build()]
            )
        ]),
        handler.config.clone()
    ));

    let resp = handler.handle_event().unwrap();
    assert_eq!((status::Ok, "commit_comment".into()), resp);
}

#[test]
fn test_issue_comment() {
    let mut handler = new_handler();
    handler.event = "issue_comment".into();
    handler.action = "created".into();
    handler.data.issue = Some(Issue {
        title: "The Issue".into(),
        html_url: "http://the-issue".into(),
        user: User::new("the-pr-owner"),
        assignees: vec![User::new("assign1"), User::new("joe-reviewer")],
    });
    handler.data.comment = Some(Comment {
        commit_id: Some("abcdef00001111".into()),
        path: Some("src/main.rs".into()),
        body: Some("I think this file should change".into()),
        html_url: "http://the-comment".into(),
        user: User::new("joe-reviewer"),
    });
    handler.data.sender = User::new("joe-reviewer");

    let attach = vec![SlackAttachmentBuilder::new("I think this file should change")
                          .title("joe.reviewer said:")
                          .title_link("http://the-comment")
                          .build()];
    let msg = "Comment on \"<http://the-issue|The Issue>\"";

    handler.messenger = Box::new(new_messenger(
        MockSlack::new(vec![
            SlackCall::new("the-reviews-channel", &format!("{} {}", msg, REPO_MSG), attach.clone()),
            SlackCall::new("@the.pr.owner", msg, attach.clone()),
            SlackCall::new("@assign1", msg, attach.clone())
        ]),
        handler.config.clone()
    ));

    let resp = handler.handle_event().unwrap();
    assert_eq!((status::Ok, "issue_comment".into()), resp);
}

#[test]
fn test_pull_request_comment() {
    let mut handler = new_handler();
    handler.event = "pull_request_review_comment".into();
    handler.action = "created".into();
    handler.data.pull_request = some_pr();
    handler.data.comment = Some(Comment {
        commit_id: Some("abcdef00001111".into()),
        path: Some("src/main.rs".into()),
        body: Some("I think this file should change".into()),
        html_url: "http://the-comment".into(),
        user: User::new("joe-reviewer"),
    });
    handler.data.sender = User::new("joe-reviewer");

    let attach = vec![SlackAttachmentBuilder::new("I think this file should change")
                          .title("joe.reviewer said:")
                          .title_link("http://the-comment")
                          .build()];
    let msg = "Comment on \"<http://the-pr|The PR>\"";

    handler.messenger = Box::new(new_messenger(
        MockSlack::new(vec![
            SlackCall::new("the-reviews-channel", &format!("{} {}", msg, REPO_MSG), attach.clone()),
            SlackCall::new("@the.pr.owner", msg, attach.clone()),
            SlackCall::new("@assign1", msg, attach.clone())
        ]),
        handler.config.clone()
    ));

    let resp = handler.handle_event().unwrap();
    assert_eq!((status::Ok, "pr_review_comment".into()), resp);
}

#[test]
fn test_pull_request_review_commented() {
    let mut handler = new_handler();
    handler.event = "pull_request_review".into();
    handler.action = "submitted".into();
    handler.data.pull_request = some_pr();
    handler.data.review = Some(Review {
        state: "commented".into(),
        body: Some("I think this file should change".into()),
        html_url: "http://the-comment".into(),
        user: User::new("joe-reviewer"),
    });
    handler.data.sender = User::new("joe-reviewer");

    let attach = vec![SlackAttachmentBuilder::new("I think this file should change")
                          .title("joe.reviewer said:")
                          .title_link("http://the-comment")
                          .build()];
    let msg = "Comment on \"<http://the-pr|The PR>\"";

    handler.messenger = Box::new(new_messenger(
        MockSlack::new(vec![
            SlackCall::new("the-reviews-channel", &format!("{} {}", msg, REPO_MSG), attach.clone()),
            SlackCall::new("@the.pr.owner", msg, attach.clone()),
            SlackCall::new("@assign1", msg, attach.clone())
        ]),
        handler.config.clone()
    ));

    let resp = handler.handle_event().unwrap();
    assert_eq!((status::Ok, "pr_review [comment]".into()), resp);
}

#[test]
fn test_pull_request_comments_ignore_empty_messages() {
    let mut handler = new_handler();
    handler.event = "pull_request_review_comment".into();
    handler.action = "created".into();
    handler.data.pull_request = some_pr();
    handler.data.comment = Some(Comment {
        commit_id: Some("abcdef00001111".into()),
        path: Some("src/main.rs".into()),
        body: Some("".into()),
        html_url: "http://the-comment".into(),
        user: User::new("joe-reviewer"),
    });
    handler.data.sender = User::new("joe-reviewer");

    // no setting of MockSlack calls --> should fail if called.

    let resp = handler.handle_event().unwrap();
    assert_eq!((status::Ok, "pr_review_comment".into()), resp);
}

#[test]
fn test_pull_request_comments_ignore_octobot() {
    let mut handler = new_handler();
    handler.event = "pull_request_review_comment".into();
    handler.action = "created".into();
    handler.data.pull_request = some_pr();
    handler.data.comment = Some(Comment {
        commit_id: Some("abcdef00001111".into()),
        path: Some("src/main.rs".into()),
        body: Some("I think this file should change".into()),
        html_url: "http://the-comment".into(),
        user: User::new("octobot"),
    });
    handler.data.sender = User::new("joe-reviewer");

    // no setting of MockSlack calls --> should fail if called.

    let resp = handler.handle_event().unwrap();
    assert_eq!((status::Ok, "pr_review_comment".into()), resp);
}

