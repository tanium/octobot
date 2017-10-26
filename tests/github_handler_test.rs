extern crate hyper;
extern crate octobot;

mod mocks;

use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use hyper::StatusCode;

use octobot::config::{Config, JiraConfig};
use octobot::force_push::ForcePushRequest;
use octobot::git_clone_manager::GitCloneManager;
use octobot::github::*;
use octobot::github::api::Session;
use octobot::jira;
use octobot::messenger;
use octobot::pr_merge::PRMergeRequest;
use octobot::repo_version::RepoVersionRequest;
use octobot::repos;
use octobot::repos::RepoConfig;
use octobot::server::github_handler::GithubEventHandler;
use octobot::slack::{self, SlackAttachmentBuilder};
use octobot::users::UserConfig;
use octobot::worker::{WorkMessage, WorkSender};

use mocks::mock_github::MockGithub;
use mocks::mock_jira::MockJira;
use mocks::mock_slack::MockSlack;

// this message gets appended only to review channel messages, not to slackbots
const REPO_MSG: &'static str = "(<http://the-github-host/some-user/some-repo|some-user/some-repo>)";

fn the_repo() -> Repo {
    Repo::parse("http://the-github-host/some-user/some-repo").unwrap()
}

struct GithubHandlerTest {
    handler: GithubEventHandler,
    github: Arc<MockGithub>,
    slack: MockSlack,
    jira: Option<Arc<MockJira>>,
    config: Arc<Config>,
    pr_merge_rx: Option<Receiver<WorkMessage<PRMergeRequest>>>,
    repo_version_rx: Option<Receiver<WorkMessage<RepoVersionRequest>>>,
    force_push_rx: Option<Receiver<WorkMessage<ForcePushRequest>>>,
}

impl GithubHandlerTest {
    fn expect_will_merge_branches(&mut self, branches: Vec<String>) -> JoinHandle<()> {
        let timeout = Duration::from_millis(300);
        let rx = self.pr_merge_rx.take().unwrap();
        thread::spawn(move || {
            for branch in branches {
                let msg = rx.recv_timeout(timeout).expect(&format!("expected to recv msg for branch: {}", branch));
                match msg {
                    WorkMessage::WorkItem(req) => {
                        assert_eq!(branch, req.target_branch);
                    }
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
    new_test_with(None)
}

fn new_test_with(jira: Option<JiraConfig>) -> GithubHandlerTest {
    let github = Arc::new(MockGithub::new());
    let slack = MockSlack::new(vec![]);
    let (pr_merge_tx, pr_merge_rx) = channel();
    let (repo_version_tx, repo_version_rx) = channel();
    let (force_push_tx, force_push_rx) = channel();

    let mut repos = RepoConfig::new();
    let mut data = HookBody::new();

    let repo_info = repos::RepoInfo::new("some-user/some-repo", "the-reviews-channel").with_jira(vec![
        "SER".to_string(),
        "CLI".to_string(),
    ]);
    repos.insert_info(github.github_host(), repo_info);

    data.repository = Repo::parse(&format!("http://{}/some-user/some-repo", github.github_host())).unwrap();
    data.sender = User::new("joe-sender");

    let mut config = Config::new(UserConfig::new(), repos);
    config.jira = jira;
    let config = Arc::new(config);

    let git_clone_manager = Arc::new(GitCloneManager::new(github.clone(), config.clone()));

    let slack_sender = slack.new_sender();

    GithubHandlerTest {
        github: github.clone(),
        slack: slack,
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
            messenger: messenger::new(config.clone(), slack_sender),
            github_session: github.clone(),
            git_clone_manager: git_clone_manager.clone(),
            jira_session: None,
            pr_merge: WorkSender::new(pr_merge_tx.clone()),
            repo_version: WorkSender::new(repo_version_tx.clone()),
            force_push: WorkSender::new(force_push_tx.clone()),
        },
    }
}

fn new_test_with_jira() -> GithubHandlerTest {
    let jira = Some(JiraConfig {
        host: "the-jira-host".into(),
        username: "the-jira-user".into(),
        password: "the-jira-pass".into(),
        progress_states: Some(vec!["the-progress".into()]),
        review_states: Some(vec!["the-review".into()]),
        resolved_states: Some(vec!["the-resolved".into()]),
        fixed_resolutions: Some(vec![":boom:".into()]),
        fix_versions_field: Some("the-versions".into()),
        pending_versions_field: Some("the-pending-versions".into()),
    });
    let mut test = new_test_with(jira);

    let jira = Arc::new(MockJira::new());
    test.jira = Some(jira.clone());
    test.handler.jira_session = Some(jira.clone());

    test
}

fn some_pr() -> Option<PullRequest> {
    Some(PullRequest {
        title: "The PR".into(),
        body: Some("The body".into()),
        number: 32,
        html_url: "http://the-pr".into(),
        state: "open".into(),
        user: User::new("the-pr-owner"),
        merged: None,
        merge_commit_sha: None,
        assignees: vec![User::new("assign1")],
        requested_reviewers: Some(vec![User::new("joe-reviewer")]),
        reviews: None,
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
            commit: CommitDetails { message: "I made a commit!".into() },
        },
        Commit {
            sha: "ffeedd00110022".into(),
            html_url: "http://commit/ffeedd00110022".into(),
            // duplicate author here to make sure dupes are removed
            author: Some(User::new("the-pr-owner")),
            commit: CommitDetails { message: "I also made a commit!".into() },
        },
    ]

}

#[test]
fn test_ping() {
    let mut test = new_test();
    test.handler.event = "ping".to_string();

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((StatusCode::Ok, "ping".into()), resp);
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

    test.slack.expect(vec![
        slack::req(
            "the-reviews-channel",
            &format!("Comment on \"src/main.rs\" (<http://the-github-host/some-user/some-repo/commit/abcdef00001111|abcdef0>) {}", REPO_MSG),
            vec![SlackAttachmentBuilder::new("I think this file should change")
                .title("joe.reviewer said:")
                .title_link("http://the-comment")
                .build()]
        )
    ]);

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((StatusCode::Ok, "commit_comment".into()), resp);
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

    test.slack.expect(vec![
        slack::req(
            "the-reviews-channel",
            &format!("Comment on \"abcdef0\" (<http://the-github-host/some-user/some-repo/commit/abcdef00001111|abcdef0>) {}", REPO_MSG),
            vec![SlackAttachmentBuilder::new("I think this file should change")
                .title("joe.reviewer said:")
                .title_link("http://the-comment")
                .build()]
        )
    ]);

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((StatusCode::Ok, "commit_comment".into()), resp);
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
        body: Some("I think this file should change, cc: @mentioned-participant".into()),
        html_url: "http://the-comment".into(),
        user: User::new("joe-reviewer"),
    });
    test.handler.data.sender = User::new("joe-reviewer");
    test.github.mock_get_pull_request_commits("some-user", "some-repo", 5, Ok(some_commits()));

    let attach = vec![
        SlackAttachmentBuilder::new("I think this file should change, cc: @mentioned-participant")
            .title("joe.reviewer said:")
            .title_link("http://the-comment")
            .build(),
    ];
    let msg = "Comment on \"<http://the-issue|The Issue>\"";

    test.slack.expect(vec![
        slack::req("the-reviews-channel", &format!("{} {}", msg, REPO_MSG), attach.clone()),
        slack::req("@the.pr.owner", msg, attach.clone()),
        slack::req("@assign1", msg, attach.clone()),
        slack::req("@bob.author", msg, attach.clone()),
        slack::req("@mentioned.participant", msg, attach.clone()),
    ]);

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((StatusCode::Ok, "issue_comment".into()), resp);
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
        body: Some("I think this file should change, cc: @mentioned-participant".into()),
        html_url: "http://the-comment".into(),
        user: User::new("joe-reviewer"),
    });
    test.handler.data.sender = User::new("joe-reviewer");
    test.github.mock_get_pull_request_commits(
        "some-user",
        "some-repo",
        32,
        Ok(some_commits()),
    );

    let attach = vec![
        SlackAttachmentBuilder::new("I think this file should change, cc: @mentioned-participant")
            .title("joe.reviewer said:")
            .title_link("http://the-comment")
            .build(),
    ];
    let msg = "Comment on \"<http://the-pr|The PR>\"";

    test.slack.expect(vec![
        slack::req("the-reviews-channel", &format!("{} {}", msg, REPO_MSG), attach.clone()),
        slack::req("@the.pr.owner", msg, attach.clone()),
        slack::req("@assign1", msg, attach.clone()),
        slack::req("@bob.author", msg, attach.clone()),
        slack::req("@mentioned.participant", msg, attach.clone()),
    ]);

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((StatusCode::Ok, "pr_review_comment".into()), resp);
}

#[test]
fn test_pull_request_review_commented() {
    let mut test = new_test();
    test.handler.event = "pull_request_review".into();
    test.handler.action = "submitted".into();
    test.handler.data.pull_request = some_pr();
    test.handler.data.review = Some(Review {
        state: "commented".into(),
        body: Some("I think this file should change, cc: @mentioned-participant".into()),
        html_url: "http://the-comment".into(),
        user: User::new("joe-reviewer"),
    });
    test.handler.data.sender = User::new("joe-reviewer");
    test.github.mock_get_pull_request_commits(
        "some-user",
        "some-repo",
        32,
        Ok(some_commits()),
    );

    let attach = vec![
        SlackAttachmentBuilder::new("I think this file should change, cc: @mentioned-participant")
            .title("joe.reviewer said:")
            .title_link("http://the-comment")
            .build(),
    ];
    let msg = "Comment on \"<http://the-pr|The PR>\"";

    test.slack.expect(vec![
        slack::req("the-reviews-channel", &format!("{} {}", msg, REPO_MSG), attach.clone()),
        slack::req("@the.pr.owner", msg, attach.clone()),
        slack::req("@assign1", msg, attach.clone()),
        slack::req("@bob.author", msg, attach.clone()),
        slack::req("@mentioned.participant", msg, attach.clone()),
    ]);

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((StatusCode::Ok, "pr_review [comment]".into()), resp);
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

    test.slack.expect(vec![]);

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((StatusCode::Ok, "pr_review_comment".into()), resp);
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
        body: Some("I think this file should change, cc: @mentioned-participant".into()),
        html_url: "http://the-comment".into(),
        user: User::new("octobot"),
    });
    test.handler.data.sender = User::new("joe-reviewer");

    test.slack.expect(vec![]);

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((StatusCode::Ok, "pr_review_comment".into()), resp);
}

#[test]
fn test_pull_request_review_approved() {
    let mut test = new_test();
    test.handler.event = "pull_request_review".into();
    test.handler.action = "submitted".into();
    test.handler.data.pull_request = some_pr();
    test.handler.data.review = Some(Review {
        state: "approved".into(),
        body: Some("I like it! cc: @mentioned-participant".into()),
        html_url: "http://the-comment".into(),
        user: User::new("joe-reviewer"),
    });
    test.handler.data.sender = User::new("joe-reviewer");
    test.github.mock_get_pull_request_commits(
        "some-user",
        "some-repo",
        32,
        Ok(some_commits()),
    );

    let attach = vec![
        SlackAttachmentBuilder::new("I like it! cc: @mentioned-participant")
            .title("Review: Approved")
            .title_link("http://the-comment")
            .color("good")
            .build(),
    ];
    let msg = "joe.reviewer approved PR \"<http://the-pr|The PR>\"";

    test.slack.expect(vec![
        slack::req("the-reviews-channel", &format!("{} {}", msg, REPO_MSG), attach.clone()),
        slack::req("@the.pr.owner", msg, attach.clone()),
        slack::req("@assign1", msg, attach.clone()),
        slack::req("@bob.author", msg, attach.clone()),
        slack::req("@mentioned.participant", msg, attach.clone()),
    ]);

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((StatusCode::Ok, "pr_review".into()), resp);
}

#[test]
fn test_pull_request_review_changes_requested() {
    let mut test = new_test();
    test.handler.event = "pull_request_review".into();
    test.handler.action = "submitted".into();
    test.handler.data.pull_request = some_pr();
    test.handler.data.review = Some(Review {
        state: "changes_requested".into(),
        body: Some("It needs some work! cc: @mentioned-participant".into()),
        html_url: "http://the-comment".into(),
        user: User::new("joe-reviewer"),
    });
    test.handler.data.sender = User::new("joe-reviewer");
    test.github.mock_get_pull_request_commits(
        "some-user",
        "some-repo",
        32,
        Ok(some_commits()),
    );

    let attach = vec![
        SlackAttachmentBuilder::new("It needs some work! cc: @mentioned-participant")
            .title("Review: Changes Requested")
            .title_link("http://the-comment")
            .color("danger")
            .build(),
    ];
    let msg = "joe.reviewer requested changes to PR \"<http://the-pr|The PR>\"";
    test.slack.expect(vec![
        slack::req("the-reviews-channel", &format!("{} {}", msg, REPO_MSG), attach.clone()),
        slack::req("@the.pr.owner", msg, attach.clone()),
        slack::req("@assign1", msg, attach.clone()),
        slack::req("@bob.author", msg, attach.clone()),
        slack::req("@mentioned.participant", msg, attach.clone()),
    ]);

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((StatusCode::Ok, "pr_review".into()), resp);
}

#[test]
fn test_pull_request_opened() {
    let mut test = new_test();
    test.handler.event = "pull_request".into();
    test.handler.action = "opened".into();
    test.handler.data.pull_request = some_pr();
    test.handler.data.sender = User::new("the-pr-owner");
    test.github.mock_get_pull_request_commits(
        "some-user",
        "some-repo",
        32,
        Ok(some_commits()),
    );

    let attach = vec![
        SlackAttachmentBuilder::new("")
            .title("Pull Request #32: \"The PR\"")
            .title_link("http://the-pr")
            .build(),
    ];
    let msg = "Pull Request opened by the.pr.owner";

    test.slack.expect(vec![
        slack::req(
            "the-reviews-channel",
            &format!("{} {}", msg, REPO_MSG),
            attach.clone()
        ),
    ]);

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((StatusCode::Ok, "pr".into()), resp);
}

#[test]
fn test_pull_request_closed() {
    let mut test = new_test();
    test.handler.event = "pull_request".into();
    test.handler.action = "closed".into();
    test.handler.data.pull_request = some_pr();
    test.handler.data.sender = User::new("the-pr-closer");
    test.github.mock_get_pull_request_commits(
        "some-user",
        "some-repo",
        32,
        Ok(some_commits()),
    );

    let attach = vec![
        SlackAttachmentBuilder::new("")
            .title("Pull Request #32: \"The PR\"")
            .title_link("http://the-pr")
            .build(),
    ];
    let msg = "Pull Request closed";

    test.slack.expect(vec![
        slack::req("the-reviews-channel", &format!("{} {}", msg, REPO_MSG), attach.clone()),
        slack::req("@the.pr.owner", msg, attach.clone()),
        slack::req("@assign1", msg, attach.clone()),
        slack::req("@bob.author", msg, attach.clone()),
        slack::req("@joe.reviewer", msg, attach.clone()),
    ]);

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((StatusCode::Ok, "pr".into()), resp);
}

#[test]
fn test_pull_request_reopened() {
    let mut test = new_test();
    test.handler.event = "pull_request".into();
    test.handler.action = "reopened".into();
    test.handler.data.pull_request = some_pr();
    test.handler.data.sender = User::new("the-pr-closer");
    test.github.mock_get_pull_request_commits(
        "some-user",
        "some-repo",
        32,
        Ok(some_commits()),
    );

    let attach = vec![
        SlackAttachmentBuilder::new("")
            .title("Pull Request #32: \"The PR\"")
            .title_link("http://the-pr")
            .build(),
    ];
    let msg = "Pull Request reopened";

    test.slack.expect(vec![
        slack::req(
            "the-reviews-channel",
            &format!("{} {}", msg, REPO_MSG),
            attach.clone()
        ),
    ]);

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((StatusCode::Ok, "pr".into()), resp);
}

#[test]
fn test_pull_request_assigned() {
    let mut test = new_test();
    test.handler.event = "pull_request".into();
    test.handler.action = "assigned".into();
    test.handler.data.pull_request = some_pr();
    if let Some(ref mut pr) = test.handler.data.pull_request {
        pr.assignees.push(User::new("assign2"));
    }
    test.handler.data.sender = User::new("the-pr-closer");
    test.github.mock_get_pull_request_commits(
        "some-user",
        "some-repo",
        32,
        Ok(some_commits()),
    );

    let attach = vec![
        SlackAttachmentBuilder::new("")
            .title("Pull Request #32: \"The PR\"")
            .title_link("http://the-pr")
            .build(),
    ];
    let msg = "Pull Request assigned to assign1, assign2";

    test.slack.expect(vec![
        slack::req("the-reviews-channel", &format!("{} {}", msg, REPO_MSG), attach.clone()),
        slack::req("@the.pr.owner", msg, attach.clone()),
        slack::req("@assign1", msg, attach.clone()),
        slack::req("@assign2", msg, attach.clone()),
        slack::req("@bob.author", msg, attach.clone()),
        slack::req("@joe.reviewer", msg, attach.clone()),
    ]);

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((StatusCode::Ok, "pr".into()), resp);
}

#[test]
fn test_pull_request_unassigned() {
    let mut test = new_test();
    test.handler.event = "pull_request".into();
    test.handler.action = "unassigned".into();
    test.handler.data.pull_request = some_pr();
    test.handler.data.sender = User::new("the-pr-closer");
    test.github.mock_get_pull_request_commits(
        "some-user",
        "some-repo",
        32,
        Ok(some_commits()),
    );

    let attach = vec![
        SlackAttachmentBuilder::new("")
            .title("Pull Request #32: \"The PR\"")
            .title_link("http://the-pr")
            .build(),
    ];
    let msg = "Pull Request unassigned";

    test.slack.expect(vec![
        slack::req(
            "the-reviews-channel",
            &format!("{} {}", msg, REPO_MSG),
            attach.clone()
        ),
    ]);

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((StatusCode::Ok, "pr".into()), resp);
}

#[test]
fn test_pull_request_review_requested() {
    let mut test = new_test();
    test.handler.event = "pull_request".into();
    test.handler.action = "review_requested".into();
    test.handler.data.pull_request = some_pr();
    if let Some(ref mut pr) = test.handler.data.pull_request {
        pr.requested_reviewers = Some(vec![User::new("joe-reviewer"), User::new("smith-reviewer")]);
    }
    test.handler.data.sender = User::new("the-pr-closer");
    test.github.mock_get_pull_request_commits(
        "some-user",
        "some-repo",
        32,
        Ok(some_commits()),
    );

    let attach = vec![
        SlackAttachmentBuilder::new("")
            .title("Pull Request #32: \"The PR\"")
            .title_link("http://the-pr")
            .build(),
    ];
    let msg = "Pull Request submitted for review to joe.reviewer, smith.reviewer";

    test.slack.expect(vec![
        slack::req("the-reviews-channel", &format!("{} {}", msg, REPO_MSG), attach.clone()),
        slack::req("@the.pr.owner", msg, attach.clone()),
        slack::req("@assign1", msg, attach.clone()),
        slack::req("@bob.author", msg, attach.clone()),
        slack::req("@joe.reviewer", msg, attach.clone()),
        slack::req("@smith.reviewer", msg, attach.clone()),
    ]);

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((StatusCode::Ok, "pr".into()), resp);
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
    assert_eq!((StatusCode::Ok, "pr".into()), resp);
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
    assert_eq!((StatusCode::Ok, "pr".into()), resp);
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

    test.github.mock_get_pull_request_commits(
        "some-user",
        "some-repo",
        32,
        Ok(some_commits()),
    );
    test.github.mock_get_pull_request_labels(
        "some-user",
        "some-repo",
        32,
        Err("whooops.".into()),
    );

    let msg1 = "Pull Request merged";
    let attach1 = vec![
        SlackAttachmentBuilder::new("")
            .title("Pull Request #32: \"The PR\"")
            .title_link("http://the-pr")
            .build(),
    ];

    let msg2 = "Error getting Pull Request labels";
    let attach2 = vec![SlackAttachmentBuilder::new("whooops.").color("danger").build()];

    test.slack.expect(vec![
        slack::req("the-reviews-channel", &format!("{} {}", msg1, REPO_MSG), attach1.clone()),
        slack::req("@the.pr.owner", msg1, attach1.clone()),
        slack::req("@assign1", msg1, attach1.clone()),
        slack::req("@bob.author", msg1, attach1.clone()),
        slack::req("@joe.reviewer", msg1, attach1.clone()),

        slack::req("the-reviews-channel", &format!("{} {}", msg2, REPO_MSG), attach2.clone()),
        slack::req("@the.pr.owner", msg2, attach2.clone()),
    ]);

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((StatusCode::Ok, "pr".into()), resp);
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

    test.github.mock_get_pull_request_commits(
        "some-user",
        "some-repo",
        32,
        Ok(some_commits()),
    );
    test.github.mock_get_pull_request_labels("some-user", "some-repo", 32, Ok(vec![]));

    let attach = vec![
        SlackAttachmentBuilder::new("")
            .title("Pull Request #32: \"The PR\"")
            .title_link("http://the-pr")
            .build(),
    ];
    let msg = "Pull Request merged";

    test.slack.expect(vec![
        slack::req("the-reviews-channel", &format!("{} {}", msg, REPO_MSG), attach.clone()),
        slack::req("@the.pr.owner", msg, attach.clone()),
        slack::req("@assign1", msg, attach.clone()),
        slack::req("@bob.author", msg, attach.clone()),
        slack::req("@joe.reviewer", msg, attach.clone()),
    ]);

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((StatusCode::Ok, "pr".into()), resp);
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

    let attach = vec![
        SlackAttachmentBuilder::new("")
            .title("Pull Request #32: \"The PR\"")
            .title_link("http://the-pr")
            .build(),
    ];
    let msg = "Pull Request merged";

    test.github.mock_get_pull_request_commits(
        "some-user",
        "some-repo",
        32,
        Ok(some_commits()),
    );
    test.github.mock_get_pull_request_labels(
        "some-user",
        "some-repo",
        32,
        Ok(vec![
            Label::new("other"),
            Label::new("backport-1.0"),
            Label::new("BACKPORT-2.0"),
            Label::new("non-matching"),
        ]),
    );

    test.slack.expect(vec![
        slack::req("the-reviews-channel", &format!("{} {}", msg, REPO_MSG), attach.clone()),
        slack::req("@the.pr.owner", msg, attach.clone()),
        slack::req("@assign1", msg, attach.clone()),
        slack::req("@bob.author", msg, attach.clone()),
        slack::req("@joe.reviewer", msg, attach.clone()),
    ]);

    let expect_thread = test.expect_will_merge_branches(vec!["release/1.0".into(), "release/2.0".into()]);

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((StatusCode::Ok, "pr".into()), resp);

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
    assert_eq!((StatusCode::Ok, "pr".into()), resp);

    expect_thread.join().unwrap();
}

#[test]
fn test_push_no_pr() {
    let mut test = new_test();
    test.handler.event = "push".into();
    test.handler.data.ref_name = Some("refs/heads/some-branch".into());
    test.handler.data.before = Some("abcdef0000".into());
    test.handler.data.after = Some("1111abcdef".into());

    test.github.mock_get_pull_requests(
        "some-user",
        "some-repo",
        Some("open".into()),
        None,
        Ok(vec![]),
    );

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((StatusCode::Ok, "push".into()), resp);
}

#[test]
fn test_push_with_pr() {
    let mut test = new_test();
    test.handler.event = "push".into();
    test.handler.data.ref_name = Some("refs/heads/some-branch".into());
    test.handler.data.before = Some("the-before-commit".into());
    test.handler.data.after = Some("the-after-commit".into());

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

    // Note: github shouldn't really let you have two PR's for a single branch.
    // This multiple PR support is theoretical only, but also allows us to test the
    // race condition between the hook event and the PRs api.
    let mut pr1 = some_pr().unwrap();
    pr1.head.sha = "the-before-commit".into();

    let mut pr2 = pr1.clone();
    pr2.head.sha = "the-after-commit".into();
    pr2.number = 99;
    pr2.assignees = vec![User::new("assign2")];
    pr2.requested_reviewers = None;

    test.github.mock_get_pull_request_commits(
        "some-user",
        "some-repo",
        32,
        Ok(some_commits()),
    );
    test.github.mock_get_pull_request_commits(
        "some-user",
        "some-repo",
        99,
        Ok(some_commits()),
    );

    test.github.mock_get_pull_requests(
        "some-user",
        "some-repo",
        Some("open".into()),
        None,
        Ok(vec![pr1, pr2]),
    );

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

    test.slack.expect(vec![
        slack::req("the-reviews-channel", &format!("{} {}", msg, REPO_MSG), attach1.clone()),
        slack::req("@the.pr.owner", msg, attach1.clone()),
        slack::req("@assign1", msg, attach1.clone()),
        slack::req("@bob.author", msg, attach1.clone()),
        slack::req("@joe.reviewer", msg, attach1.clone()),

        slack::req("the-reviews-channel", &format!("{} {}", msg, REPO_MSG), attach2.clone()),
        slack::req("@the.pr.owner", msg, attach2.clone()),
        slack::req("@assign2", msg, attach2.clone()),
        slack::req("@bob.author", msg, attach2.clone()),
    ]);

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((StatusCode::Ok, "push".into()), resp);
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

    let mut pr = some_pr().unwrap();
    pr.head.sha = "abcdef0000".into();
    test.github.mock_get_pull_requests(
        "some-user",
        "some-repo",
        Some("open".into()),
        None,
        Ok(vec![pr]),
    );
    test.github.mock_get_pull_request_commits(
        "some-user",
        "some-repo",
        32,
        Ok(some_commits()),
    );

    let msg = "joe.sender pushed 0 commit(s) to branch some-branch";
    let attach = vec![
        SlackAttachmentBuilder::new("")
            .title("Pull Request #32: \"The PR\"")
            .title_link("http://the-pr")
            .build(),
    ];
    test.slack.expect(vec![
        slack::req("the-reviews-channel", &format!("{} {}", msg, REPO_MSG), attach.clone()),
        slack::req("@the.pr.owner", msg, attach.clone()),
        slack::req("@assign1", msg, attach.clone()),
        slack::req("@bob.author", msg, attach.clone()),
        slack::req("@joe.reviewer", msg, attach.clone()),
    ]);

    // Setup background thread to validate force-push msg
    let expect_thread;
    {
        let timeout = Duration::from_millis(300);
        let rx = test.force_push_rx.take().expect("force-push message");
        expect_thread = thread::spawn(move || {
            let msg = rx.recv_timeout(timeout).expect(&format!("expected to recv msg"));
            match msg {
                WorkMessage::WorkItem(req) => {
                    assert_eq!("abcdef0000", req.before_hash);
                    assert_eq!("1111abcdef", req.after_hash);
                }
                _ => {
                    panic!("Unexpected messages: {:?}", msg);
                }
            };

            let last_message = rx.recv_timeout(timeout);
            assert!(last_message.is_err());
        });
    }

    let resp = test.handler.handle_event().expect("handled event");
    assert_eq!((StatusCode::Ok, "push".into()), resp);

    expect_thread.join().expect("expect-thread finished");
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
    pr.head.sha = "abcdef0000".into();
    pr.title = "WIP: Awesome new feature".into();
    test.github.mock_get_pull_requests(
        "some-user",
        "some-repo",
        Some("open".into()),
        None,
        Ok(vec![pr]),
    );
    test.github.mock_get_pull_request_commits(
        "some-user",
        "some-repo",
        32,
        Ok(some_commits()),
    );

    // Note: not slack expectations here. It should not notify slack for WIP PRs.

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
    assert_eq!((StatusCode::Ok, "push".into()), resp);

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
    test.handler.data.repository = Repo::parse(
        &format!("http://{}/some-other-user/some-other-repo", test.github.github_host()),
    ).unwrap();

    let mut pr = some_pr().unwrap();
    pr.head.sha = "1111abcdef".into();
    test.github.mock_get_pull_requests(
        "some-other-user",
        "some-other-repo",
        Some("open"),
        None,
        Ok(vec![pr]),
    );
    test.github.mock_get_pull_request_commits(
        "some-other-user",
        "some-other-repo",
        32,
        Ok(some_commits()),
    );

    let msg = "joe.sender pushed 0 commit(s) to branch some-branch";
    let attach = vec![
        SlackAttachmentBuilder::new("")
            .title("Pull Request #32: \"The PR\"")
            .title_link("http://the-pr")
            .build(),
    ];
    test.slack.expect(vec![
        slack::req("@the.pr.owner", msg, attach.clone()),
        slack::req("@assign1", msg, attach.clone()),
        slack::req("@bob.author", msg, attach.clone()),
        slack::req("@joe.reviewer", msg, attach.clone()),
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
    assert_eq!((StatusCode::Ok, "push".into()), resp);

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
            commit: CommitDetails { message: "Fix [SER-1] Add the feature\n\nThe body ([OTHER-123])".into() },
        },
    ]
}

fn some_jira_push_commits() -> Vec<PushCommit> {
    vec![
        PushCommit {
            id: "ffeedd00110011".into(),
            tree_id: "ffeedd00110011".into(),
            url: "http://commit/ffeedd00110011".into(),
            message: "Fix [SER-1] Add the feature\n\nThe body ([OTHER-123])".into(),
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

    test.github.mock_get_pull_request_commits(
        "some-user",
        "some-repo",
        32,
        Ok(some_jira_commits()),
    );

    let attach = vec![
        SlackAttachmentBuilder::new("")
            .title("Pull Request #32: \"The PR\"")
            .title_link("http://the-pr")
            .build(),
    ];
    let msg = "Pull Request opened by the.pr.owner";

    test.slack.expect(vec![
        slack::req(
            "the-reviews-channel",
            &format!("{} {}", msg, REPO_MSG),
            attach.clone()
        ),
    ]);

    if let Some(ref jira) = test.jira {
        jira.mock_comment_issue(
            "SER-1",
            "Review submitted for branch master: http://the-pr",
            Ok(()),
        );

        jira.mock_get_transitions("SER-1", Ok(vec![new_transition("001", "the-progress")]));
        jira.mock_transition_issue("SER-1", &new_transition_req("001"), Ok(()));

        jira.mock_get_transitions("SER-1", Ok(vec![new_transition("002", "the-review")]));
        jira.mock_transition_issue("SER-1", &new_transition_req("002"), Ok(()));
    }

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((StatusCode::Ok, "pr".into()), resp);
}

#[test]
fn test_jira_push_master() {
    let mut test = new_test_with_jira();
    test.handler.event = "push".into();
    test.handler.data.ref_name = Some("refs/heads/master".into());
    test.handler.data.before = Some("abcdef0000".into());
    test.handler.data.after = Some("1111abcdef".into());
    test.handler.data.commits = Some(some_jira_push_commits());

    if let Some(ref jira) = test.jira {
        jira.mock_comment_issue("SER-1", "Merged into branch master: [ffeedd0|http://commit/ffeedd00110011]\n{quote}Fix [SER-1] Add the feature{quote}", Ok(()));

        jira.mock_get_transitions("SER-1", Ok(vec![new_transition("003", "the-resolved")]));
        jira.mock_transition_issue("SER-1", &new_transition_req("003"), Ok(()));
    }

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((StatusCode::Ok, "push".into()), resp);
}

#[test]
fn test_jira_push_develop() {
    let mut test = new_test_with_jira();
    test.handler.event = "push".into();
    test.handler.data.ref_name = Some("refs/heads/develop".into());
    test.handler.data.before = Some("abcdef0000".into());
    test.handler.data.after = Some("1111abcdef".into());
    test.handler.data.commits = Some(some_jira_push_commits());

    if let Some(ref jira) = test.jira {
        jira.mock_comment_issue("SER-1", "Merged into branch develop: [ffeedd0|http://commit/ffeedd00110011]\n{quote}Fix [SER-1] Add the feature{quote}", Ok(()));

        jira.mock_get_transitions("SER-1", Ok(vec![new_transition("003", "the-resolved")]));
        jira.mock_transition_issue("SER-1", &new_transition_req("003"), Ok(()));
    }

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((StatusCode::Ok, "push".into()), resp);
}

#[test]
fn test_jira_push_release() {
    let mut test = new_test_with_jira();
    test.handler.event = "push".into();
    test.handler.data.ref_name = Some("refs/heads/release/55".into());
    test.handler.data.before = Some("abcdef0000".into());
    test.handler.data.after = Some("1111abcdef".into());
    test.handler.data.commits = Some(some_jira_push_commits());

    if let Some(ref jira) = test.jira {
        jira.mock_comment_issue("SER-1", "Merged into branch release/55: [ffeedd0|http://commit/ffeedd00110011]\n{quote}Fix [SER-1] Add the feature{quote}", Ok(()));

        jira.mock_get_transitions("SER-1", Ok(vec![new_transition("003", "the-resolved")]));
        jira.mock_transition_issue("SER-1", &new_transition_req("003"), Ok(()));
    }

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((StatusCode::Ok, "push".into()), resp);
}

#[test]
fn test_jira_push_other_branch() {
    let mut test = new_test_with_jira();
    test.handler.event = "push".into();
    test.handler.data.ref_name = Some("refs/heads/some-branch".into());
    test.handler.data.before = Some("abcdef0000".into());
    test.handler.data.after = Some("1111abcdef".into());

    test.handler.data.commits = Some(some_jira_push_commits());

    test.github.mock_get_pull_requests(
        "some-user",
        "some-repo",
        Some("open".into()),
        None,
        Ok(vec![]),
    );

    // no jira mocks: will fail if called

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((StatusCode::Ok, "push".into()), resp);
}


#[test]
fn test_jira_disabled() {
    let mut test = new_test_with_jira();
    test.handler.event = "push".into();
    test.handler.data.ref_name = Some("refs/heads/master".into());
    test.handler.data.before = Some("abcdef0000".into());
    test.handler.data.after = Some("1111abcdef".into());
    test.handler.data.commits = Some(some_jira_push_commits());

    // change the repo to an unconfigured one
    test.handler.data.repository = Repo::parse(
        &format!("http://{}/some-other-user/some-other-repo", test.github.github_host()),
    ).unwrap();

    // no jira mocks: will fail if called

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((StatusCode::Ok, "push".into()), resp);
}

#[test]
fn test_jira_push_triggers_version_script() {
    let mut test = new_test_with_jira();

    test.config.repos_write().insert_info(
        test.github.github_host(),
        repos::RepoInfo::new("some-user/versioning-repo", "the-reviews-channel")
            .with_version_script(Some("echo 1.2.3.4".into())),
    );

    // change the repo to an unconfigured one
    test.handler.data.repository = Repo::parse(
        &format!("http://{}/some-user/versioning-repo", test.github.github_host()),
    ).unwrap();

    test.handler.event = "push".into();
    test.handler.data.ref_name = Some("refs/heads/master".into());
    test.handler.data.before = Some("abcdef0000".into());
    test.handler.data.after = Some("1111abcdef".into());
    test.handler.data.commits = Some(some_jira_push_commits());

    // Setup background thread to validate version msg
    // Note: no expectations are set on mock_jira since we have stubbed out the background worker thread
    let expect_thread;
    {
        let timeout = Duration::from_millis(300);
        let rx = test.repo_version_rx.take().unwrap();
        expect_thread = thread::spawn(move || {
            let msg = rx.recv_timeout(timeout).expect(&format!("expected to recv msg"));
            match msg {
                WorkMessage::WorkItem(req) => {
                    assert_eq!("master", req.branch);
                    assert_eq!("1111abcdef", req.commit_hash);
                }
                _ => {
                    panic!("Unexpected messages: {:?}", msg);
                }
            };

            let last_message = rx.recv_timeout(timeout);
            assert!(last_message.is_err());
        });
    }

    let resp = test.handler.handle_event().unwrap();
    assert_eq!((StatusCode::Ok, "push".into()), resp);

    expect_thread.join().unwrap()
}
