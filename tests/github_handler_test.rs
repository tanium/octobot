extern crate iron;
extern crate octobot;

mod mocks;

use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;
use std::thread::{self, JoinHandle};

use iron::status;

use octobot::config::{Config, JiraConfig};
use octobot::repos::RepoConfig;
use octobot::users::UserConfig;
use octobot::github::*;
use octobot::github::api::Session;
use octobot::git_clone_manager::GitCloneManager;
use octobot::jira;
use octobot::messenger::SlackMessenger;
use octobot::slack::SlackAttachmentBuilder;
use octobot::server::github_handler::GithubEventHandler;
use octobot::pr_merge::PRMergeMessage;
use octobot::repo_version::RepoVersionMessage;
use octobot::force_push::ForcePushMessage;
use octobot::repos;

use mocks::mock_github::MockGithub;
use mocks::mock_jira::MockJira;
use mocks::mock_slack::{SlackCall, MockSlack};

// this message gets appended only to review channel messages, not to slackbots
const REPO_MSG : &'static str = "(<http://the-github-host/some-user/some-repo|some-user/some-repo>)";

fn the_repo() -> Repo {
    Repo::parse("http://the-github-host/some-user/some-repo").unwrap()
}

struct GithubHandlerTest {
    handler: GithubEventHandler,
    github: Arc<MockGithub>,
    jira: Option<Arc<MockJira>>,
    config: Arc<Config>,
    pr_merge_rx: Option<Receiver<PRMergeMessage>>,
    repo_version_rx: Option<Receiver<RepoVersionMessage>>,
    force_push_rx: Option<Receiver<ForcePushMessage>>,
}

impl GithubHandlerTest {
    fn expect_slack_calls(&mut self, calls: Vec<SlackCall>) {
        self.handler.messenger = Box::new(SlackMessenger {
            config: self.config.clone(),
            slack: Rc::new(MockSlack::new(calls)),
        });
    }

    fn expect_will_merge_branches(&mut self, branches: Vec<String>) -> JoinHandle<()> {
        let timeout = Duration::from_millis(300);
        let rx = self.pr_merge_rx.take().unwrap();
        thread::spawn(move || {
            for branch in branches {
                let msg = rx.recv_timeout(timeout).expect(&format!("expected to recv msg for branch: {}", branch));
                match msg {
                    PRMergeMessage::Merge(req) => {
                        assert_eq!(branch, req.target_branch);
                    },
                    _ => {
                        panic!("Unexpected messages: {:?}", msg);
                    }
                };
           }

            let last_message = rx.recv_timeout(timeout);
            assert!(last_message.is_err());
        })
    }
}

fn new_test() -> GithubHandlerTest {
    let github = Arc::new(MockGithub::new());
    let slack = Rc::new(MockSlack::new(vec![]));
    let (pr_merge_tx, pr_merge_rx) = channel();
    let (repo_version_tx, repo_version_rx) = channel();
    let (force_push_tx, force_push_rx) = channel();

    let mut repos = RepoConfig::new();
    let mut data = HookBody::new();

    repos.insert(github.github_host(),
                 "some-user/some-repo",
                 "the-reviews-channel");
    data.repository = Repo::parse(&format!("http://{}/some-user/some-repo", github.github_host()))
        .unwrap();
    data.sender = User::new("joe-sender");

    let config = Arc::new(Config::new(UserConfig::new(), repos));
    let git_clone_manager = Arc::new(GitCloneManager::new(github.clone(), config.clone()));

    GithubHandlerTest {
        github: github.clone(),
        jira: None,
        config: config.clone(),
        pr_merge_rx: Some(pr_merge_rx),
        repo_version_rx: Some(repo_version_rx),
        force_push_rx: Some(force_push_rx),
        handler: GithubEventHandler {
            event: "ping".to_string(),
            data: data,
            action: "".to_string(),
            config: config.clone(),
            messenger: Box::new(SlackMessenger {
                config: config.clone(),
                slack: slack.clone(),
            }),
            github_session: github.clone(),
            git_clone_manager: git_clone_manager.clone(),
            jira_session: None,
            pr_merge: pr_merge_tx.clone(),
            repo_version: repo_version_tx.clone(),
            force_push: force_push_tx.clone(),
        },
    }
}

fn new_test_with_jira() -> GithubHandlerTest {
    let mut test = new_test();

    {
        let mut config: Config = (*test.config).clone();
        config.jira = Some(JiraConfig {
            host: "the-jira-host".into(),
            username: "the-jira-user".into(),
            password: "the-jira-pass".into(),
            progress_states: Some(vec!["the-progress".into()]),
            review_states: Some(vec!["the-review".into()]),
            resolved_states: Some(vec!["the-resolved".into()]),
            fixed_resolutions: Some(vec![":boom:".into()]),
        });
        test.config = Arc::new(config);
        test.handler.config = test.config.clone();

        let jira = Arc::new(MockJira::new());
        test.jira = Some(jira.clone());
        test.handler.jira_session = Some(jira.clone());
    }

    test
}

fn some_pr() -> Option<PullRequest> {
    Some(PullRequest {
        title: "The PR".into(),
        body: "The body".into(),
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

fn some_commits() -> Vec<Commit> {
    vec![
        Commit {
            sha: "ffeedd00110011".into(),
            html_url: "http://commit/ffeedd00110011".into(),
            author: Some(User::new("bob-author")),
            commit: CommitDetails {
                message: "I made a commit!".into(),
            }
        },
        Commit {
            sha: "ffeedd00110022".into(),
            html_url: "http://commit/ffeedd00110022".into(),
            // duplicate author here to make sure dupes are removed
            author: Some(User::new("the-pr-owner")),
            commit: CommitDetails {
                message: "I also made a commit!".into(),
            }
        },
    ]

}

#[test]
fn test_ping() {
    let mut test = new_test();
    test.handler.event = "ping".to_string();

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((status::Ok, "ping".into()), resp);
}

#[test]
fn test_commit_comment_with_path() {
    let mut test = new_test();
    test.handler.event = "commit_comment".into();
    test.handler.action = "created".into();
    test.handler.data.comment = Some(Comment {
        commit_id: Some("abcdef00001111".into()),
        path: Some("src/main.rs".into()),
        body: Some("I think this file should change".into()),
        html_url: "http://the-comment".into(),
        user: User::new("joe-reviewer"),
    });
    test.handler.data.sender = User::new("joe-reviewer");

    test.expect_slack_calls(vec![
        SlackCall::new(
            "the-reviews-channel",
            &format!("Comment on \"src/main.rs\" (<http://the-github-host/some-user/some-repo/commit/abcdef00001111|abcdef0>) {}", REPO_MSG),
            vec![SlackAttachmentBuilder::new("I think this file should change")
                .title("joe.reviewer said:")
                .title_link("http://the-comment")
                .build()]
        )
    ]);

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((status::Ok, "commit_comment".into()), resp);
}

#[test]
fn test_commit_comment_no_path() {
    let mut test = new_test();
    test.handler.event = "commit_comment".into();
    test.handler.action = "created".into();
    test.handler.data.comment = Some(Comment {
        commit_id: Some("abcdef00001111".into()),
        path: None,
        body: Some("I think this file should change".into()),
        html_url: "http://the-comment".into(),
        user: User::new("joe-reviewer"),
    });
    test.handler.data.sender = User::new("joe-reviewer");

    test.expect_slack_calls(vec![
        SlackCall::new(
            "the-reviews-channel",
            &format!("Comment on \"abcdef0\" (<http://the-github-host/some-user/some-repo/commit/abcdef00001111|abcdef0>) {}", REPO_MSG),
            vec![SlackAttachmentBuilder::new("I think this file should change")
                .title("joe.reviewer said:")
                .title_link("http://the-comment")
                .build()]
        )
    ]);

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((status::Ok, "commit_comment".into()), resp);
}

#[test]
fn test_issue_comment() {
    let mut test = new_test();
    test.handler.event = "issue_comment".into();
    test.handler.action = "created".into();
    test.handler.data.issue = Some(Issue {
        number: 5,
        title: "The Issue".into(),
        html_url: "http://the-issue".into(),
        user: User::new("the-pr-owner"),
        assignees: vec![User::new("assign1"), User::new("joe-reviewer")],
    });
    test.handler.data.comment = Some(Comment {
        commit_id: Some("abcdef00001111".into()),
        path: Some("src/main.rs".into()),
        body: Some("I think this file should change".into()),
        html_url: "http://the-comment".into(),
        user: User::new("joe-reviewer"),
    });
    test.handler.data.sender = User::new("joe-reviewer");
    test.github.mock_get_pull_request_commits("some-user", "some-repo", 5, Ok(some_commits()));

    let attach = vec![SlackAttachmentBuilder::new("I think this file should change")
                          .title("joe.reviewer said:")
                          .title_link("http://the-comment")
                          .build()];
    let msg = "Comment on \"<http://the-issue|The Issue>\"";

    test.expect_slack_calls(vec![
        SlackCall::new("the-reviews-channel", &format!("{} {}", msg, REPO_MSG), attach.clone()),
        SlackCall::new("@the.pr.owner", msg, attach.clone()),
        SlackCall::new("@assign1", msg, attach.clone()),
        SlackCall::new("@bob.author", msg, attach.clone()),
    ]);

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((status::Ok, "issue_comment".into()), resp);
}

#[test]
fn test_pull_request_comment() {
    let mut test = new_test();
    test.handler.event = "pull_request_review_comment".into();
    test.handler.action = "created".into();
    test.handler.data.pull_request = some_pr();
    test.handler.data.comment = Some(Comment {
        commit_id: Some("abcdef00001111".into()),
        path: Some("src/main.rs".into()),
        body: Some("I think this file should change".into()),
        html_url: "http://the-comment".into(),
        user: User::new("joe-reviewer"),
    });
    test.handler.data.sender = User::new("joe-reviewer");
    test.github.mock_get_pull_request_commits("some-user", "some-repo", 32, Ok(some_commits()));

    let attach = vec![SlackAttachmentBuilder::new("I think this file should change")
                          .title("joe.reviewer said:")
                          .title_link("http://the-comment")
                          .build()];
    let msg = "Comment on \"<http://the-pr|The PR>\"";

    test.expect_slack_calls(vec![
        SlackCall::new("the-reviews-channel", &format!("{} {}", msg, REPO_MSG), attach.clone()),
        SlackCall::new("@the.pr.owner", msg, attach.clone()),
        SlackCall::new("@assign1", msg, attach.clone()),
        SlackCall::new("@bob.author", msg, attach.clone()),
    ]);

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((status::Ok, "pr_review_comment".into()), resp);
}

#[test]
fn test_pull_request_review_commented() {
    let mut test = new_test();
    test.handler.event = "pull_request_review".into();
    test.handler.action = "submitted".into();
    test.handler.data.pull_request = some_pr();
    test.handler.data.review = Some(Review {
        state: "commented".into(),
        body: Some("I think this file should change".into()),
        html_url: "http://the-comment".into(),
        user: User::new("joe-reviewer"),
    });
    test.handler.data.sender = User::new("joe-reviewer");
    test.github.mock_get_pull_request_commits("some-user", "some-repo", 32, Ok(some_commits()));

    let attach = vec![SlackAttachmentBuilder::new("I think this file should change")
                          .title("joe.reviewer said:")
                          .title_link("http://the-comment")
                          .build()];
    let msg = "Comment on \"<http://the-pr|The PR>\"";

    test.expect_slack_calls(vec![
        SlackCall::new("the-reviews-channel", &format!("{} {}", msg, REPO_MSG), attach.clone()),
        SlackCall::new("@the.pr.owner", msg, attach.clone()),
        SlackCall::new("@assign1", msg, attach.clone()),
        SlackCall::new("@bob.author", msg, attach.clone()),
    ]);

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((status::Ok, "pr_review [comment]".into()), resp);
}

#[test]
fn test_pull_request_comments_ignore_empty_messages() {
    let mut test = new_test();
    test.handler.event = "pull_request_review_comment".into();
    test.handler.action = "created".into();
    test.handler.data.pull_request = some_pr();
    test.handler.data.comment = Some(Comment {
        commit_id: Some("abcdef00001111".into()),
        path: Some("src/main.rs".into()),
        body: Some("".into()),
        html_url: "http://the-comment".into(),
        user: User::new("joe-reviewer"),
    });
    test.handler.data.sender = User::new("joe-reviewer");

    test.expect_slack_calls(vec![]);

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((status::Ok, "pr_review_comment".into()), resp);
}

#[test]
fn test_pull_request_comments_ignore_octobot() {
    let mut test = new_test();
    test.handler.event = "pull_request_review_comment".into();
    test.handler.action = "created".into();
    test.handler.data.pull_request = some_pr();
    test.handler.data.comment = Some(Comment {
        commit_id: Some("abcdef00001111".into()),
        path: Some("src/main.rs".into()),
        body: Some("I think this file should change".into()),
        html_url: "http://the-comment".into(),
        user: User::new("octobot"),
    });
    test.handler.data.sender = User::new("joe-reviewer");

    test.expect_slack_calls(vec![]);

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((status::Ok, "pr_review_comment".into()), resp);
}

#[test]
fn test_pull_request_review_approved() {
    let mut test = new_test();
    test.handler.event = "pull_request_review".into();
    test.handler.action = "submitted".into();
    test.handler.data.pull_request = some_pr();
    test.handler.data.review = Some(Review {
        state: "approved".into(),
        body: Some("I like it!".into()),
        html_url: "http://the-comment".into(),
        user: User::new("joe-reviewer"),
    });
    test.handler.data.sender = User::new("joe-reviewer");
    test.github.mock_get_pull_request_commits("some-user", "some-repo", 32, Ok(some_commits()));

    let attach = vec![SlackAttachmentBuilder::new("I like it!")
                          .title("Review: Approved")
                          .title_link("http://the-comment")
                          .color("good")
                          .build()];
    let msg = "joe.reviewer approved PR \"<http://the-pr|The PR>\"";

    test.expect_slack_calls(vec![
        SlackCall::new("the-reviews-channel", &format!("{} {}", msg, REPO_MSG), attach.clone()),
        SlackCall::new("@the.pr.owner", msg, attach.clone()),
        SlackCall::new("@assign1", msg, attach.clone()),
        SlackCall::new("@bob.author", msg, attach.clone()),
    ]);

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((status::Ok, "pr_review".into()), resp);
}

#[test]
fn test_pull_request_review_changes_requested() {
    let mut test = new_test();
    test.handler.event = "pull_request_review".into();
    test.handler.action = "submitted".into();
    test.handler.data.pull_request = some_pr();
    test.handler.data.review = Some(Review {
        state: "changes_requested".into(),
        body: Some("It needs some work!".into()),
        html_url: "http://the-comment".into(),
        user: User::new("joe-reviewer"),
    });
    test.handler.data.sender = User::new("joe-reviewer");
    test.github.mock_get_pull_request_commits("some-user", "some-repo", 32, Ok(some_commits()));

    let attach = vec![SlackAttachmentBuilder::new("It needs some work!")
                          .title("Review: Changes Requested")
                          .title_link("http://the-comment")
                          .color("danger")
                          .build()];
    let msg = "joe.reviewer requested changes to PR \"<http://the-pr|The PR>\"";
    test.expect_slack_calls(vec![
        SlackCall::new("the-reviews-channel", &format!("{} {}", msg, REPO_MSG), attach.clone()),
        SlackCall::new("@the.pr.owner", msg, attach.clone()),
        SlackCall::new("@assign1", msg, attach.clone()),
        SlackCall::new("@bob.author", msg, attach.clone()),
    ]);

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((status::Ok, "pr_review".into()), resp);
}

#[test]
fn test_pull_request_opened() {
    let mut test = new_test();
    test.handler.event = "pull_request".into();
    test.handler.action = "opened".into();
    test.handler.data.pull_request = some_pr();
    test.handler.data.sender = User::new("the-pr-owner");
    test.github.mock_get_pull_request_commits("some-user", "some-repo", 32, Ok(some_commits()));

    let attach = vec![SlackAttachmentBuilder::new("")
                          .title("Pull Request #32: \"The PR\"")
                          .title_link("http://the-pr")
                          .build()];
    let msg = "Pull Request opened by the.pr.owner";

    test.expect_slack_calls(vec![
        SlackCall::new("the-reviews-channel", &format!("{} {}", msg, REPO_MSG), attach.clone()),
        SlackCall::new("@assign1", msg, attach.clone()),
        SlackCall::new("@bob.author", msg, attach.clone()),
        SlackCall::new("@joe.reviewer", msg, attach.clone()),
    ]);

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((status::Ok, "pr".into()), resp);
}

#[test]
fn test_pull_request_closed() {
    let mut test = new_test();
    test.handler.event = "pull_request".into();
    test.handler.action = "closed".into();
    test.handler.data.pull_request = some_pr();
    test.handler.data.sender = User::new("the-pr-closer");
    test.github.mock_get_pull_request_commits("some-user", "some-repo", 32, Ok(some_commits()));

    let attach = vec![SlackAttachmentBuilder::new("")
                          .title("Pull Request #32: \"The PR\"")
                          .title_link("http://the-pr")
                          .build()];
    let msg = "Pull Request closed";

    test.expect_slack_calls(vec![
        SlackCall::new("the-reviews-channel", &format!("{} {}", msg, REPO_MSG), attach.clone()),
        SlackCall::new("@the.pr.owner", msg, attach.clone()),
        SlackCall::new("@assign1", msg, attach.clone()),
        SlackCall::new("@bob.author", msg, attach.clone()),
        SlackCall::new("@joe.reviewer", msg, attach.clone()),
    ]);

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((status::Ok, "pr".into()), resp);
}

#[test]
fn test_pull_request_reopened() {
    let mut test = new_test();
    test.handler.event = "pull_request".into();
    test.handler.action = "reopened".into();
    test.handler.data.pull_request = some_pr();
    test.handler.data.sender = User::new("the-pr-closer");
    test.github.mock_get_pull_request_commits("some-user", "some-repo", 32, Ok(some_commits()));

    let attach = vec![SlackAttachmentBuilder::new("")
                          .title("Pull Request #32: \"The PR\"")
                          .title_link("http://the-pr")
                          .build()];
    let msg = "Pull Request reopened";

    test.expect_slack_calls(vec![
        SlackCall::new("the-reviews-channel", &format!("{} {}", msg, REPO_MSG), attach.clone()),
        SlackCall::new("@the.pr.owner", msg, attach.clone()),
        SlackCall::new("@assign1", msg, attach.clone()),
        SlackCall::new("@bob.author", msg, attach.clone()),
        SlackCall::new("@joe.reviewer", msg, attach.clone()),
    ]);

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((status::Ok, "pr".into()), resp);
}

#[test]
fn test_pull_request_assigned() {
    let mut test = new_test();
    test.handler.event = "pull_request".into();
    test.handler.action = "assigned".into();
    test.handler.data.pull_request = some_pr();
    test.handler.data.sender = User::new("the-pr-closer");
    test.github.mock_get_pull_request_commits("some-user", "some-repo", 32, Ok(some_commits()));

    let attach = vec![SlackAttachmentBuilder::new("")
                          .title("Pull Request #32: \"The PR\"")
                          .title_link("http://the-pr")
                          .build()];
    let msg = "Pull Request assigned to assign1, joe.reviewer";

    test.expect_slack_calls(vec![
        SlackCall::new("the-reviews-channel", &format!("{} {}", msg, REPO_MSG), attach.clone()),
        SlackCall::new("@the.pr.owner", msg, attach.clone()),
        SlackCall::new("@assign1", msg, attach.clone()),
        SlackCall::new("@bob.author", msg, attach.clone()),
        SlackCall::new("@joe.reviewer", msg, attach.clone()),
    ]);

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((status::Ok, "pr".into()), resp);
}

#[test]
fn test_pull_request_unassigned() {
    let mut test = new_test();
    test.handler.event = "pull_request".into();
    test.handler.action = "unassigned".into();
    test.handler.data.pull_request = some_pr();
    test.handler.data.sender = User::new("the-pr-closer");
    test.github.mock_get_pull_request_commits("some-user", "some-repo", 32, Ok(some_commits()));

    let attach = vec![SlackAttachmentBuilder::new("")
                          .title("Pull Request #32: \"The PR\"")
                          .title_link("http://the-pr")
                          .build()];
    let msg = "Pull Request unassigned";

    test.expect_slack_calls(vec![
        SlackCall::new("the-reviews-channel", &format!("{} {}", msg, REPO_MSG), attach.clone()),
        SlackCall::new("@the.pr.owner", msg, attach.clone()),
        SlackCall::new("@assign1", msg, attach.clone()),
        SlackCall::new("@bob.author", msg, attach.clone()),
        SlackCall::new("@joe.reviewer", msg, attach.clone()),
    ]);

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((status::Ok, "pr".into()), resp);
}

#[test]
fn test_pull_request_other() {
    let mut test = new_test();
    test.handler.event = "pull_request".into();
    test.handler.action = "some-other-action".into();
    test.handler.data.pull_request = some_pr();
    test.handler.data.sender = User::new("the-pr-closer");

    // should not do anything!

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((status::Ok, "pr".into()), resp);
}


#[test]
fn test_pull_request_labeled_not_merged() {
    let mut test = new_test();
    test.handler.event = "pull_request".into();
    test.handler.action = "labeled".into();
    test.handler.data.pull_request = some_pr();
    if let Some(ref mut pr) = test.handler.data.pull_request {
        pr.merged = Some(false);
    }
    test.handler.data.sender = User::new("the-pr-owner");

    // labeled but not merged --> noop

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((status::Ok, "pr".into()), resp);
}

#[test]
fn test_pull_request_merged_error_getting_labels() {
    let mut test = new_test();
    test.handler.event = "pull_request".into();
    test.handler.action = "closed".into();
    test.handler.data.pull_request = some_pr();
    if let Some(ref mut pr) = test.handler.data.pull_request {
        pr.merged = Some(true);
    }
    test.handler.data.sender = User::new("the-pr-merger");

    test.github.mock_get_pull_request_commits("some-user", "some-repo", 32, Ok(some_commits()));
    test.github.mock_get_pull_request_labels("some-user", "some-repo", 32, Err("whooops.".into()));

    let msg1 = "Pull Request merged";
    let attach1 = vec![
        SlackAttachmentBuilder::new("")
          .title("Pull Request #32: \"The PR\"")
          .title_link("http://the-pr")
          .build()
    ];

    let msg2 = "Error getting Pull Request labels";
    let attach2 = vec![
        SlackAttachmentBuilder::new("whooops.")
            .color("danger")
            .build()
    ];

    test.expect_slack_calls(vec![
        SlackCall::new("the-reviews-channel", &format!("{} {}", msg1, REPO_MSG), attach1.clone()),
        SlackCall::new("@the.pr.owner", msg1, attach1.clone()),
        SlackCall::new("@assign1", msg1, attach1.clone()),
        SlackCall::new("@bob.author", msg1, attach1.clone()),
        SlackCall::new("@joe.reviewer", msg1, attach1.clone()),

        SlackCall::new("the-reviews-channel", &format!("{} {}", msg2, REPO_MSG), attach2.clone()),
        SlackCall::new("@the.pr.owner", msg2, attach2.clone()),
    ]);

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((status::Ok, "pr".into()), resp);
}

#[test]
fn test_pull_request_merged_no_labels() {
    let mut test = new_test();
    test.handler.event = "pull_request".into();
    test.handler.action = "closed".into();
    test.handler.data.pull_request = some_pr();
    if let Some(ref mut pr) = test.handler.data.pull_request {
        pr.merged = Some(true);
    }
    test.handler.data.sender = User::new("the-pr-merger");

    test.github.mock_get_pull_request_commits("some-user", "some-repo", 32, Ok(some_commits()));
    test.github.mock_get_pull_request_labels("some-user", "some-repo", 32, Ok(vec![]));

    let attach = vec![SlackAttachmentBuilder::new("")
                          .title("Pull Request #32: \"The PR\"")
                          .title_link("http://the-pr")
                          .build()];
    let msg = "Pull Request merged";

    test.expect_slack_calls(vec![
        SlackCall::new("the-reviews-channel", &format!("{} {}", msg, REPO_MSG), attach.clone()),
        SlackCall::new("@the.pr.owner", msg, attach.clone()),
        SlackCall::new("@assign1", msg, attach.clone()),
        SlackCall::new("@bob.author", msg, attach.clone()),
        SlackCall::new("@joe.reviewer", msg, attach.clone()),
    ]);

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((status::Ok, "pr".into()), resp);
}

#[test]
fn test_pull_request_merged_backport_labels() {
    let mut test = new_test();
    test.handler.event = "pull_request".into();
    test.handler.action = "closed".into();
    test.handler.data.pull_request = some_pr();
    if let Some(ref mut pr) = test.handler.data.pull_request {
        pr.merged = Some(true);
    }
    test.handler.data.sender = User::new("the-pr-merger");

    let attach = vec![SlackAttachmentBuilder::new("")
                          .title("Pull Request #32: \"The PR\"")
                          .title_link("http://the-pr")
                          .build()];
    let msg = "Pull Request merged";

    test.github.mock_get_pull_request_commits("some-user", "some-repo", 32, Ok(some_commits()));
    test.github.mock_get_pull_request_labels("some-user", "some-repo", 32, Ok(vec![
        Label::new("other"),
        Label::new("backport-1.0"),
        Label::new("BACKPORT-2.0"),
        Label::new("non-matching"),
    ]));

    test.expect_slack_calls(vec![
        SlackCall::new("the-reviews-channel", &format!("{} {}", msg, REPO_MSG), attach.clone()),
        SlackCall::new("@the.pr.owner", msg, attach.clone()),
        SlackCall::new("@assign1", msg, attach.clone()),
        SlackCall::new("@bob.author", msg, attach.clone()),
        SlackCall::new("@joe.reviewer", msg, attach.clone()),
    ]);

    let expect_thread = test.expect_will_merge_branches(vec!["release/1.0".into(), "release/2.0".into()]);

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((status::Ok, "pr".into()), resp);

    expect_thread.join().unwrap();
}

#[test]
fn test_pull_request_merged_retroactively_labeled() {
    let mut test = new_test();
    test.handler.event = "pull_request".into();
    test.handler.action = "labeled".into();
    test.handler.data.pull_request = some_pr();
    if let Some(ref mut pr) = test.handler.data.pull_request {
        pr.merged = Some(true);
    }
    test.handler.data.label = Some(Label::new("backport-7.123"));
    test.handler.data.sender = User::new("the-pr-merger");

    let expect_thread = test.expect_will_merge_branches(vec!["release/7.123".into()]);

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((status::Ok, "pr".into()), resp);

    expect_thread.join().unwrap();
}

#[test]
fn test_push_no_pr() {
    let mut test = new_test();
    test.handler.event = "push".into();
    test.handler.data.ref_name = Some("refs/heads/some-branch".into());
    test.handler.data.before = Some("abcdef0000".into());
    test.handler.data.after = Some("1111abcdef".into());

    test.github.mock_get_pull_requests("some-user", "some-repo", Some("open".into()), Some("1111abcdef"), Ok(vec![]));

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((status::Ok, "push".into()), resp);
}

#[test]
fn test_push_with_pr() {
    let mut test = new_test();
    test.handler.event = "push".into();
    test.handler.data.ref_name = Some("refs/heads/some-branch".into());
    test.handler.data.before = Some("abcdef0000".into());
    test.handler.data.after = Some("1111abcdef".into());

    test.handler.data.commits = Some(vec![
        PushCommit {
            id: "aaaaaa000000".into(),
            tree_id: "".into(),
            message: "add stuff".into(),
            url: "http://commit1".into(),
        },
        PushCommit {
            id: "1111abcdef".into(),
            tree_id: "".into(),
            message: "fix stuff".into(),
            url: "http://commit2".into(),
        },
    ]);

    let pr1 = some_pr().unwrap();
    let mut pr2 = pr1.clone();
    pr2.number = 99;
    pr2.assignees = vec![User::new("assign2")];

    test.github.mock_get_pull_request_commits("some-user", "some-repo", 32, Ok(some_commits()));
    test.github.mock_get_pull_request_commits("some-user", "some-repo", 99, Ok(some_commits()));

    test.github.mock_get_pull_requests("some-user", "some-repo", Some("open".into()), Some("1111abcdef"), Ok(vec![pr1, pr2]));

    let msg = "joe.sender pushed 2 commit(s) to branch some-branch";
    let attach_common = vec![
        SlackAttachmentBuilder::new("<http://commit1|aaaaaa0>: add stuff").build(),
        SlackAttachmentBuilder::new("<http://commit2|1111abc>: fix stuff").build(),
    ];

    let mut attach1 = vec![
        SlackAttachmentBuilder::new("")
            .title("Pull Request #32: \"The PR\"")
            .title_link("http://the-pr")
            .build(),
    ];
    attach1.append(&mut attach_common.clone());

    let mut attach2 = vec![
        SlackAttachmentBuilder::new("")
            .title("Pull Request #99: \"The PR\"")
            .title_link("http://the-pr")
            .build(),
    ];
    attach2.append(&mut attach_common.clone());

    test.expect_slack_calls(vec![
        SlackCall::new("the-reviews-channel", &format!("{} {}", msg, REPO_MSG), attach1.clone()),
        SlackCall::new("@the.pr.owner", msg, attach1.clone()),
        SlackCall::new("@assign1", msg, attach1.clone()),
        SlackCall::new("@bob.author", msg, attach1.clone()),
        SlackCall::new("@joe.reviewer", msg, attach1.clone()),

        SlackCall::new("the-reviews-channel", &format!("{} {}", msg, REPO_MSG), attach2.clone()),
        SlackCall::new("@the.pr.owner", msg, attach2.clone()),
        SlackCall::new("@assign2", msg, attach2.clone()),
        SlackCall::new("@bob.author", msg, attach2.clone()),
    ]);

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((status::Ok, "push".into()), resp);
}

#[test]
fn test_push_force_notify() {
    let mut test = new_test();

    test.handler.event = "push".into();
    test.handler.data.ref_name = Some("refs/heads/some-branch".into());
    test.handler.data.before = Some("abcdef0000".into());
    test.handler.data.after = Some("1111abcdef".into());
    test.handler.data.forced = Some(true);
    test.handler.data.compare = Some("http://compare-url".into());

    let pr = some_pr().unwrap();
    test.github.mock_get_pull_requests("some-user", "some-repo", Some("open".into()), Some("1111abcdef"), Ok(vec![pr]));
    test.github.mock_get_pull_request_commits("some-user", "some-repo", 32, Ok(some_commits()));

    let msg = "joe.sender pushed 0 commit(s) to branch some-branch";
    let attach = vec![
        SlackAttachmentBuilder::new("")
            .title("Pull Request #32: \"The PR\"")
            .title_link("http://the-pr")
            .build(),
    ];
    test.expect_slack_calls(vec![
        SlackCall::new("the-reviews-channel", &format!("{} {}", msg, REPO_MSG), attach.clone()),
        SlackCall::new("@the.pr.owner", msg, attach.clone()),
        SlackCall::new("@assign1", msg, attach.clone()),
        SlackCall::new("@bob.author", msg, attach.clone()),
        SlackCall::new("@joe.reviewer", msg, attach.clone()),
    ]);

    // Setup background thread to validate force-push msg
    let expect_thread;
    {
        let timeout = Duration::from_millis(300);
        let rx = test.force_push_rx.take().unwrap();
        expect_thread = thread::spawn(move || {
            let msg = rx.recv_timeout(timeout).expect(&format!("expected to recv msg"));
            match msg {
                ForcePushMessage::Check(req) => {
                    assert_eq!("abcdef0000", req.before_hash);
                    assert_eq!("1111abcdef", req.after_hash);
                },
                _ => {
                    panic!("Unexpected messages: {:?}", msg);
                }
            };

            let last_message = rx.recv_timeout(timeout);
            assert!(last_message.is_err());
        });
    }

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((status::Ok, "push".into()), resp);

    expect_thread.join().unwrap();
}

#[test]
fn test_push_force_notify_wip() {
    let mut test = new_test();

    test.handler.event = "push".into();
    test.handler.data.ref_name = Some("refs/heads/some-branch".into());
    test.handler.data.before = Some("abcdef0000".into());
    test.handler.data.after = Some("1111abcdef".into());
    test.handler.data.forced = Some(true);

    let mut pr = some_pr().unwrap();
    pr.title = "WIP: Awesome new feature".into();
    test.github.mock_get_pull_requests("some-user", "some-repo", Some("open".into()), Some("1111abcdef"), Ok(vec![pr]));
    test.github.mock_get_pull_request_commits("some-user", "some-repo", 32, Ok(some_commits()));

    let msg = "joe.sender pushed 0 commit(s) to branch some-branch";
    let attach = vec![
        SlackAttachmentBuilder::new("")
            .title("Pull Request #32: \"WIP: Awesome new feature\"")
            .title_link("http://the-pr")
            .build(),
    ];
    test.expect_slack_calls(vec![
        SlackCall::new("the-reviews-channel", &format!("{} {}", msg, REPO_MSG), attach.clone()),
        SlackCall::new("@the.pr.owner", msg, attach.clone()),
        SlackCall::new("@assign1", msg, attach.clone()),
        SlackCall::new("@bob.author", msg, attach.clone()),
        SlackCall::new("@joe.reviewer", msg, attach.clone()),
    ]);

    // Setup background thread to validate force-push msg
    let expect_thread;
    {
        let timeout = Duration::from_millis(300);
        let rx = test.force_push_rx.take().unwrap();
        expect_thread = thread::spawn(move || {
            let last_message = rx.recv_timeout(timeout);
            assert!(last_message.is_err());
        });
    }

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((status::Ok, "push".into()), resp);

    expect_thread.join().unwrap();
}

#[test]
fn test_push_force_notify_ignored() {
    let mut test = new_test();

    test.handler.event = "push".into();
    test.handler.data.ref_name = Some("refs/heads/some-branch".into());
    test.handler.data.before = Some("abcdef0000".into());
    test.handler.data.after = Some("1111abcdef".into());
    test.handler.data.forced = Some(true);

    // change the repo to an unconfigured one
    test.handler.data.repository =
        Repo::parse(&format!("http://{}/some-other-user/some-other-repo", test.github.github_host())).unwrap();

    let pr = some_pr().unwrap();
    test.github.mock_get_pull_requests("some-other-user", "some-other-repo", Some("open"), Some("1111abcdef"), Ok(vec![pr]));
    test.github.mock_get_pull_request_commits("some-other-user", "some-other-repo", 32, Ok(some_commits()));

    let msg = "joe.sender pushed 0 commit(s) to branch some-branch";
    let attach = vec![
        SlackAttachmentBuilder::new("")
            .title("Pull Request #32: \"The PR\"")
            .title_link("http://the-pr")
            .build(),
    ];
    test.expect_slack_calls(vec![
        SlackCall::new("@the.pr.owner", msg, attach.clone()),
        SlackCall::new("@assign1", msg, attach.clone()),
        SlackCall::new("@bob.author", msg, attach.clone()),
        SlackCall::new("@joe.reviewer", msg, attach.clone()),
    ]);

    // Setup background thread to validate force-push msg
    let expect_thread;
    {
        let timeout = Duration::from_millis(300);
        let rx = test.force_push_rx.take().unwrap();
        expect_thread = thread::spawn(move || {
            let last_message = rx.recv_timeout(timeout);
            assert!(last_message.is_err());
        });
    }

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((status::Ok, "push".into()), resp);

    expect_thread.join().unwrap();
}

fn new_transition(id: &str, name: &str) -> jira::Transition {
    jira::Transition {
        id: id.into(),
        name: name.into(),
        to: jira::TransitionTo {
            id: String::new(),
            name: format!("{}-inner", name),
        },
        fields: None,
    }
}

fn new_transition_req(id: &str) -> jira::TransitionRequest {
    jira::TransitionRequest {
        transition: jira::IDOrName {
            id: Some(id.into()),
            name: None,
        },
        fields: None,
    }
}

fn some_jira_commits() -> Vec<Commit> {
    vec![
        Commit {
            sha: "ffeedd00110011".into(),
            html_url: "http://commit/ffeedd00110011".into(),
            author: Some(User::new("bob-author")),
            commit: CommitDetails {
                message: "Fix [SER-1] Add the feature\n\nThe body".into(),
            }
        },
    ]
}

fn some_jira_push_commits() -> Vec<PushCommit> {
    vec![
        PushCommit {
            id: "ffeedd00110011".into(),
            tree_id: "ffeedd00110011".into(),
            url: "http://commit/ffeedd00110011".into(),
            message: "Fix [SER-1] Add the feature\n\nThe body".into(),
        },
    ]
}
#[test]
fn test_jira_pull_request_opened() {
    let mut test = new_test_with_jira();
    test.handler.event = "pull_request".into();
    test.handler.action = "opened".into();
    test.handler.data.pull_request = some_pr();
    test.handler.data.sender = User::new("the-pr-owner");

    test.github.mock_get_pull_request_commits("some-user", "some-repo", 32, Ok(some_jira_commits()));

    let attach = vec![SlackAttachmentBuilder::new("")
                          .title("Pull Request #32: \"The PR\"")
                          .title_link("http://the-pr")
                          .build()];
    let msg = "Pull Request opened by the.pr.owner";

    test.expect_slack_calls(vec![
        SlackCall::new("the-reviews-channel", &format!("{} {}", msg, REPO_MSG), attach.clone()),
        SlackCall::new("@assign1", msg, attach.clone()),
        SlackCall::new("@bob.author", msg, attach.clone()),
        SlackCall::new("@joe.reviewer", msg, attach.clone()),
    ]);

    if let Some(ref jira) = test.jira {
        jira.mock_comment_issue("SER-1", "Review submitted for branch master: http://the-pr", Ok(()));

        jira.mock_get_transitions("SER-1", Ok(vec![new_transition("001", "the-progress")]));
        jira.mock_transition_issue("SER-1", &new_transition_req("001"), Ok(()));

        jira.mock_get_transitions("SER-1", Ok(vec![new_transition("002", "the-review")]));
        jira.mock_transition_issue("SER-1", &new_transition_req("002"), Ok(()));
    }

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((status::Ok, "pr".into()), resp);
}

#[test]
fn test_jira_push_master() {
    let mut test = new_test_with_jira();
    test.handler.event = "push".into();
    test.handler.data.ref_name = Some("refs/heads/master".into());
    test.handler.data.before = Some("abcdef0000".into());
    test.handler.data.after = Some("1111abcdef".into());
    test.handler.data.commits = Some(some_jira_push_commits());

    test.github.mock_get_pull_requests("some-user", "some-repo", Some("open".into()), Some("1111abcdef"), Ok(vec![]));

    if let Some(ref jira) = test.jira {
        jira.mock_comment_issue("SER-1", "Merged into branch master: [ffeedd0|http://commit/ffeedd00110011]\n{quote}Fix [SER-1] Add the feature{quote}", Ok(()));

        jira.mock_get_transitions("SER-1", Ok(vec![new_transition("003", "the-resolved")]));
        jira.mock_transition_issue("SER-1", &new_transition_req("003"), Ok(()));
    }

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((status::Ok, "push".into()), resp);
}

#[test]
fn test_jira_push_develop() {
    let mut test = new_test_with_jira();
    test.handler.event = "push".into();
    test.handler.data.ref_name = Some("refs/heads/develop".into());
    test.handler.data.before = Some("abcdef0000".into());
    test.handler.data.after = Some("1111abcdef".into());
    test.handler.data.commits = Some(some_jira_push_commits());

    test.github.mock_get_pull_requests("some-user", "some-repo", Some("open".into()), Some("1111abcdef"), Ok(vec![]));

    if let Some(ref jira) = test.jira {
        jira.mock_comment_issue("SER-1", "Merged into branch develop: [ffeedd0|http://commit/ffeedd00110011]\n{quote}Fix [SER-1] Add the feature{quote}", Ok(()));

        jira.mock_get_transitions("SER-1", Ok(vec![new_transition("003", "the-resolved")]));
        jira.mock_transition_issue("SER-1", &new_transition_req("003"), Ok(()));
    }

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((status::Ok, "push".into()), resp);
}

#[test]
fn test_jira_push_release() {
    let mut test = new_test_with_jira();
    test.handler.event = "push".into();
    test.handler.data.ref_name = Some("refs/heads/release/55".into());
    test.handler.data.before = Some("abcdef0000".into());
    test.handler.data.after = Some("1111abcdef".into());
    test.handler.data.commits = Some(some_jira_push_commits());

    test.github.mock_get_pull_requests("some-user", "some-repo", Some("open".into()), Some("1111abcdef"), Ok(vec![]));

    if let Some(ref jira) = test.jira {
        jira.mock_comment_issue("SER-1", "Merged into branch release/55: [ffeedd0|http://commit/ffeedd00110011]\n{quote}Fix [SER-1] Add the feature{quote}", Ok(()));

        jira.mock_get_transitions("SER-1", Ok(vec![new_transition("003", "the-resolved")]));
        jira.mock_transition_issue("SER-1", &new_transition_req("003"), Ok(()));
    }

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((status::Ok, "push".into()), resp);
}

#[test]
fn test_jira_push_other_branch() {
    let mut test = new_test_with_jira();
    test.handler.event = "push".into();
    test.handler.data.ref_name = Some("refs/heads/some-branch".into());
    test.handler.data.before = Some("abcdef0000".into());
    test.handler.data.after = Some("1111abcdef".into());

    test.handler.data.commits = Some(some_jira_push_commits());

    test.github.mock_get_pull_requests("some-user", "some-repo", Some("open".into()), Some("1111abcdef"), Ok(vec![]));

    // no jira mocks: will fail if called

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((status::Ok, "push".into()), resp);
}


#[test]
fn test_jira_disabled() {
    let mut test = new_test_with_jira();
    test.handler.event = "push".into();
    test.handler.data.ref_name = Some("refs/heads/master".into());
    test.handler.data.before = Some("abcdef0000".into());
    test.handler.data.after = Some("1111abcdef".into());
    test.handler.data.commits = Some(some_jira_push_commits());

    test.github.mock_get_pull_requests("some-other-user", "some-other-repo", Some("open".into()), Some("1111abcdef"), Ok(vec![]));

    // change the repo to an unconfigured one
    test.handler.data.repository =
        Repo::parse(&format!("http://{}/some-other-user/some-other-repo", test.github.github_host())).unwrap();

    // no jira mocks: will fail if called

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((status::Ok, "push".into()), resp);
}

#[test]
fn test_jira_push_triggers_version_script() {
    let mut test = new_test_with_jira();

    let mut config: Config = (*test.config).clone();
    config.repos.insert_info(test.github.github_host(), "some-user/versioning-repo",
                             repos::RepoInfo::new("the-reviews-channel").with_version_script(Some(vec!["echo".into(), "1.2.3.4".into()])));

    test.config = Arc::new(config);
    test.handler.config = test.config.clone();

    // change the repo to an unconfigured one
    test.handler.data.repository =
        Repo::parse(&format!("http://{}/some-user/versioning-repo", test.github.github_host())).unwrap();

    test.handler.event = "push".into();
    test.handler.data.ref_name = Some("refs/heads/master".into());
    test.handler.data.before = Some("abcdef0000".into());
    test.handler.data.after = Some("1111abcdef".into());
    test.handler.data.commits = Some(some_jira_push_commits());

    test.github.mock_get_pull_requests("some-user", "versioning-repo", Some("open".into()), Some("1111abcdef"), Ok(vec![]));

    // Setup background thread to validate version msg
    // Note: no expectations are set on mock_jira since we have stubbed out the background worker thread
    let expect_thread;
    {
        let timeout = Duration::from_millis(300);
        let rx = test.repo_version_rx.take().unwrap();
        expect_thread = thread::spawn(move || {
            let msg = rx.recv_timeout(timeout).expect(&format!("expected to recv msg"));
            match msg {
                RepoVersionMessage::Version(req) => {
                    assert_eq!("master", req.branch);
                    assert_eq!("1111abcdef", req.commit_hash);
                },
                _ => {
                    panic!("Unexpected messages: {:?}", msg);
                }
            };

            let last_message = rx.recv_timeout(timeout);
            assert!(last_message.is_err());
        });
    }

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((status::Ok, "push".into()), resp);

    expect_thread.join().unwrap()
}
