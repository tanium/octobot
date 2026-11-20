use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use hyper::StatusCode;
use tempfile::{TempDir, tempdir};
use tokio::sync::broadcast;

use mocks::mock_github::MockGithub;
use mocks::mock_jira::MockJira;
use mocks::mock_slack::MockSlack;
use mocks::mock_worker::LockedMockWorker;
use octobot::server::commits_cache::CommitsCache;
use octobot::server::github_handler::{GithubEventHandler, TeamsCache};
use octobot::server::people_cache::PeopleCache;
use octobot::server::reviewers_cache::ReviewersCache;
use octobot_events::Kind;
use octobot_lib::config::{Config, JiraAuth, JiraConfig};
use octobot_lib::config_db::ConfigDatabase;
use octobot_lib::github::api::Session;
use octobot_lib::github::*;
use octobot_lib::jira::{self};
use octobot_lib::metrics::Metrics;
use octobot_lib::repos;
use octobot_lib::slack::SlackRecipient;
use octobot_ops::event_bus::{Change, Event, EventBus};
use octobot_ops::force_push::{self, ForcePushRequest};
use octobot_ops::messenger;
use octobot_ops::pr_merge::{self, PRMergeRequest};
use octobot_ops::repo_version::{self, RepoVersionRequest};
use octobot_ops::review_group::ReviewGroups;
use octobot_ops::slack::{self, SlackAttachmentBuilder, ThreadRoot, Threading};

mod mocks;

// this message gets appended only to review channel messages, not to slackbots
const REPO_MSG: &str = "(<http://the-github-host/some-user/some-repo|some-user/some-repo>)";

fn thread_root(repo_msg: &str, number: u32) -> ThreadRoot {
    ThreadRoot {
        msg: format!("New thread for Pull Request {}", repo_msg),
        attachments: vec![
            SlackAttachmentBuilder::new("")
                .title(format!("Pull Request #{}: \"The PR\"", number))
                .title_link("http://the-pr")
                .build(),
        ],
    }
}

fn dm_thread_root(number: u32) -> ThreadRoot {
    let root = thread_root(REPO_MSG, number);
    ThreadRoot {
        msg: format!(
            "New thread for Pull Request #{}: \"The PR\" {}",
            number, REPO_MSG
        ),
        ..root
    }
}

fn the_repo() -> Repo {
    Repo::parse("http://the-github-host/some-user/some-repo").unwrap()
}

struct GithubHandlerTest {
    handler: GithubEventHandler,
    github: Arc<MockGithub>,
    slack: MockSlack,
    jira: Option<Arc<MockJira>>,
    _temp_dir: TempDir,
    config: Arc<Config>,
    pr_merge: LockedMockWorker<PRMergeRequest>,
    repo_version: LockedMockWorker<RepoVersionRequest>,
    force_push: LockedMockWorker<ForcePushRequest>,
}

impl GithubHandlerTest {
    fn expect_will_merge_branches(
        &mut self,
        release_branch_prefix: &str,
        branches: Vec<String>,
        commits: Vec<Commit>,
    ) {
        let repo = &self.handler.repository;
        let pr = &self.handler.data.pull_request.as_ref().unwrap();

        for branch in branches {
            self.pr_merge.expect_req(pr_merge::req(
                repo,
                pr,
                &branch,
                release_branch_prefix,
                &commits,
            ));
        }
    }

    fn expect_will_force_push_notify(
        &mut self,
        pr: &PullRequest,
        before_hash: &str,
        after_hash: &str,
    ) {
        let repo = &self.handler.repository;

        self.force_push
            .expect_req(force_push::req(repo, pr, before_hash, after_hash));
    }

    fn expect_will_run_version_script(
        &mut self,
        branch: &str,
        commit_hash: &str,
        commits: &[PushCommit],
    ) {
        let repo = &self.handler.repository;

        self.repo_version
            .expect_req(repo_version::req(repo, branch, commit_hash, commits));
    }

    fn mock_pull_request_commits(&self) -> Vec<Commit> {
        let commits = some_commits();
        self.github.mock_get_pull_request_commits(
            "some-user",
            "some-repo",
            32,
            Ok(commits.clone()),
        );

        commits
    }

    fn mock_pull_request_reviews(&self, reviews: Vec<Review>) {
        self.github
            .mock_get_pull_request_reviews("some-user", "some-repo", 32, Ok(reviews));
    }

    fn subscribe_events(&mut self, kind: Kind) -> broadcast::Receiver<Arc<Event>> {
        let bus = Arc::new(EventBus::new(10, Metrics::new()));
        let receiver = bus.subscribe();
        self.handler.messenger = messenger::new(self.config.clone(), self.slack.new_sender())
            .with_events(bus, kind, &self.handler.action);
        receiver
    }

    fn mock_get_team_members(&self, team_id: u32) -> Vec<User> {
        let users = some_members();
        self.github
            .mock_get_team_members(&the_repo(), team_id, Ok(users.clone()));

        users
    }
}

fn new_test() -> GithubHandlerTest {
    new_test_with(None)
}

fn new_test_with(jira: Option<JiraConfig>) -> GithubHandlerTest {
    let github = Arc::new(MockGithub::new());
    let slack = MockSlack::new(vec![]);
    let pr_merge = LockedMockWorker::new("pr-merge");
    let repo_version = LockedMockWorker::new("repo-version");
    let force_push = LockedMockWorker::new("force-push");

    let temp_dir = tempdir().unwrap();
    let db_file = temp_dir.path().join("db.sqlite3");
    let db = ConfigDatabase::new(&db_file.to_string_lossy()).expect("create temp database");

    let mut data = HookBody::new();

    let repository = Repo::parse(&format!(
        "http://{}/some-user/some-repo",
        github.github_host()
    ))
    .unwrap();
    data.sender = User::new("joe-sender");

    let mut config = Config::new(db);

    config
        .users_write()
        .insert("the-pr-owner", "the.pr.owner")
        .unwrap();
    config
        .users_write()
        .insert("joe-sender", "joe.sender")
        .unwrap();
    config
        .users_write()
        .insert("joe-reviewer", "joe.reviewer")
        .unwrap();
    config
        .users_write()
        .insert("smith-reviewer", "smith.reviewer")
        .unwrap();
    config.users_write().insert("assign1", "assign1").unwrap();
    config.users_write().insert("assign2", "assign2").unwrap();
    config
        .users_write()
        .insert("bob-author", "bob.author")
        .unwrap();
    config
        .users_write()
        .insert("mentioned-participant", "mentioned.participant")
        .unwrap();
    config
        .users_write()
        .insert("team-member1", "team.member1")
        .unwrap();
    config
        .users_write()
        .insert("team-member2", "team.member2")
        .unwrap();

    config
        .repos_write()
        .insert_info(
            &repos::RepoInfo::new("some-user/some-repo", "the-reviews-channel")
                .with_jira("SER")
                .with_jira("CLI")
                .with_use_threads(true)
                .with_force_push(true),
        )
        .expect("Failed to add some-user/some-repo");

    config.slack.ignored_users = vec!["ignore-me[bot]".into()];
    config.jira = jira;
    let config = Arc::new(config);

    let slack_sender = slack.new_sender();
    let pr_merge_sender = pr_merge.new_sender();
    let repo_version_sender = repo_version.new_sender();
    let force_push_sender = force_push.new_sender();

    GithubHandlerTest {
        github: github.clone(),
        slack,
        jira: None,
        _temp_dir: temp_dir,
        config: config.clone(),
        pr_merge,
        repo_version,
        force_push,
        handler: GithubEventHandler {
            event: "ping".to_string(),
            data,
            repository,
            action: "".to_string(),
            config: config.clone(),
            messenger: messenger::new(config, slack_sender),
            github_session: github,
            jira_session: None,
            pr_merge: pr_merge_sender,
            repo_version: repo_version_sender,
            force_push: force_push_sender,
            team_members_cache: Arc::new(TeamsCache::new(Duration::new(3600, 0))),
            reviewers_cache: Arc::new(ReviewersCache::new(Duration::new(3600, 0))),
            commits_cache: Arc::new(CommitsCache::new(Duration::new(3600, 0))),
            people_cache: Arc::new(PeopleCache::new(Duration::new(3600, 0))),
            metrics: Metrics::new(),
        },
    }
}

fn new_test_with_jira() -> GithubHandlerTest {
    let jira = Some(JiraConfig {
        base_url: "http://the-jira-host".into(),
        auth: JiraAuth::Basic {
            username: "the-jira-user".into(),
            password: "the-jira-pass".into(),
        },
        progress_states: vec!["the-progress".into()],
        review_states: vec!["the-review".into()],
        resolved_states: vec!["the-resolved".into()],
        fixed_resolutions: vec![":boom:".into()],
        fix_versions_field: "the-versions".into(),
        frozen_states: vec![],
        pending_versions_field: Some("the-pending-versions".into()),
        restrict_comment_visibility_to_role: None,
    });
    let mut test = new_test_with(jira);

    let jira = Arc::new(MockJira::new());
    test.jira = Some(jira.clone());
    test.handler.jira_session = Some(jira);

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
        requested_teams: None,
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
        draft: None,
        labels: None,
        updated_at: Some("2026-11-13T10:00:00Z".into()),
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
            },
        },
        Commit {
            sha: "ffeedd00110022".into(),
            html_url: "http://commit/ffeedd00110022".into(),
            // duplicate author here to make sure dupes are removed
            author: Some(User::new("the-pr-owner")),
            commit: CommitDetails {
                message: "I also made a commit!".into(),
            },
        },
    ]
}

fn some_members() -> Vec<User> {
    vec![User::new("team-member1"), User::new("team-member2")]
}

fn expect_jira_ref_fail(git: &MockGithub) {
    expect_jira_ref_fail_pr(git, some_pr().as_ref().unwrap(), &some_commits())
}

fn expect_jira_ref_fail_pr(git: &MockGithub, pr: &PullRequest, commits: &[Commit]) {
    let mut run =
        CheckRun::new("jira", &commits.last().unwrap().sha, None).completed(Conclusion::Neutral);
    run.output = Some(CheckOutput::new("Missing JIRA reference", ""));

    git.mock_create_check_run(pr, &run, Ok(1));
}

fn expect_jira_ref_pass_pr(git: &MockGithub, pr: &PullRequest, commits: &[Commit]) {
    git.mock_create_check_run(
        pr,
        &CheckRun::new("jira", &commits.last().unwrap().sha, None).completed(Conclusion::Success),
        Ok(1),
    );
}

#[tokio::test]
async fn test_ping() {
    let mut test = new_test();
    test.handler.event = "ping".to_string();

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "ping".into()), resp);
}

// `handles()` and the dispatch table are generated from the same macro table,
// so they cannot drift; this just sanity-checks the generated predicate.
#[test]
fn test_handles() {
    assert!(GithubEventHandler::handles("ping"));
    assert!(GithubEventHandler::handles("push"));
    assert!(GithubEventHandler::handles("check_run"));
    assert!(!GithubEventHandler::handles("workflow_run"));
    assert!(!GithubEventHandler::handles("status"));
}

#[tokio::test]
async fn test_commit_comment_with_path() {
    let mut test = new_test();
    test.handler.event = "commit_comment".into();
    test.handler.action = "created".into();
    test.handler.data.comment = Some(Comment {
        id: 0,
        pull_request_review_id: None,
        commit_id: Some("abcdef00001111".into()),
        path: Some("src/main.rs".into()),
        body: Some("I think this file should change".into()),
        html_url: "http://the-comment".into(),
        user: User::new("joe-reviewer"),
    });
    test.handler.data.sender = User::new("joe-reviewer");
    let mut pr1 = some_pr().unwrap();
    pr1.head.sha = "the-before-commit".into();

    let mut pr2 = pr1.clone();
    pr2.head.sha = "the-after-commit".into();
    pr2.number = 99;
    pr2.assignees = vec![User::new("assign2")];
    pr2.requested_reviewers = None;
    test.github.mock_get_pull_requests_by_commit(
        "some-user",
        "some-repo",
        "abcdef0",
        None,
        Ok(vec![pr1.clone(), pr2.clone()]),
    );
    test.slack.expect(vec![
        slack::req(
            SlackRecipient::by_name("the-reviews-channel"),
            &format!("Comment on \"src/main.rs\" (<http://the-github-host/some-user/some-repo/commit/abcdef00001111|abcdef0>) {}", REPO_MSG),
            &[SlackAttachmentBuilder::new("I think this file should change")
                .title("joe.reviewer said:")
                .title_link("http://the-comment")
                .build()],
            Threading::ReplyOrStart("some-user/some-repo/32".to_string(), thread_root(REPO_MSG, 32)),
        ),
        slack::req(
            SlackRecipient::by_name("the-reviews-channel"),
            &format!("Comment on \"src/main.rs\" (<http://the-github-host/some-user/some-repo/commit/abcdef00001111|abcdef0>) {}", REPO_MSG),
            &[SlackAttachmentBuilder::new("I think this file should change")
                .title("joe.reviewer said:")
                .title_link("http://the-comment")
                .build()],
            Threading::ReplyOrStart("some-user/some-repo/99".to_string(), thread_root(REPO_MSG, 99)),
        ),
    ]);

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "commit_comment".into()), resp);
}

#[tokio::test]
async fn test_commit_comment_with_path_that_is_included_in_multiple_prs() {
    let mut test = new_test();
    test.handler.event = "commit_comment".into();
    test.handler.action = "created".into();
    test.handler.data.comment = Some(Comment {
        id: 0,
        pull_request_review_id: None,
        commit_id: Some("abcdef00001111".into()),
        path: Some("src/main.rs".into()),
        body: Some("I think this file should change".into()),
        html_url: "http://the-comment".into(),
        user: User::new("joe-reviewer"),
    });
    test.handler.data.sender = User::new("joe-reviewer");
    let pr = some_pr();
    test.github.mock_get_pull_requests_by_commit(
        "some-user",
        "some-repo",
        "abcdef0",
        None,
        Ok(vec![pr.clone().unwrap()]),
    );
    test.slack.expect(vec![
        slack::req(
            SlackRecipient::by_name("the-reviews-channel"),
            &format!("Comment on \"src/main.rs\" (<http://the-github-host/some-user/some-repo/commit/abcdef00001111|abcdef0>) {}", REPO_MSG),
            &[SlackAttachmentBuilder::new("I think this file should change")
                .title("joe.reviewer said:")
                .title_link("http://the-comment")
                .build()],
            Threading::ReplyOrStart("some-user/some-repo/32".to_string(), thread_root(REPO_MSG, 32)),
        )
    ]);

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "commit_comment".into()), resp);
}

#[tokio::test]
async fn test_commit_comment_no_path() {
    let mut test = new_test();
    test.handler.event = "commit_comment".into();
    test.handler.action = "created".into();
    test.handler.data.comment = Some(Comment {
        id: 0,
        pull_request_review_id: None,
        commit_id: Some("abcdef00001111".into()),
        path: None,
        body: Some("I think this file should change".into()),
        html_url: "http://the-comment".into(),
        user: User::new("joe-reviewer"),
    });
    test.handler.data.sender = User::new("joe-reviewer");

    test.github.mock_get_pull_requests_by_commit(
        "some-user",
        "some-repo",
        "abcdef0",
        None,
        Ok(vec![]),
    );
    test.slack.expect(vec![
        slack::req(
            SlackRecipient::by_name("the-reviews-channel"),
            &format!("Comment on \"abcdef0\" (<http://the-github-host/some-user/some-repo/commit/abcdef00001111|abcdef0>) {}", REPO_MSG),
            &[SlackAttachmentBuilder::new("I think this file should change")
                .title("joe.reviewer said:")
                .title_link("http://the-comment")
                .build()],
            Threading::Flat,
        )
    ]);

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "commit_comment".into()), resp);
}

#[tokio::test]
async fn test_issue_comment() {
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
        id: 0,
        pull_request_review_id: None,
        commit_id: Some("abcdef00001111".into()),
        path: Some("src/main.rs".into()),
        body: Some("I think this file should change, cc: @mentioned-participant".into()),
        html_url: "http://the-comment".into(),
        user: User::new("joe-reviewer"),
    });
    test.handler.data.sender = User::new("joe-reviewer");

    let attach = vec![
        SlackAttachmentBuilder::new("I think this file should change, cc: @mentioned-participant")
            .title("joe.reviewer said:")
            .title_link("http://the-comment")
            .build(),
    ];
    let msg = "Comment on \"<http://the-issue|The Issue>\"";

    test.slack.expect(vec![
        slack::req(
            SlackRecipient::by_name("the-reviews-channel"),
            &format!("{} {}", msg, REPO_MSG),
            &attach,
            Threading::Reply("some-user/some-repo/5".to_string()),
        ),
        slack::req(
            SlackRecipient::user_mention("assign1"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("mentioned.participant"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("the.pr.owner"),
            msg,
            &attach,
            Threading::Flat,
        ),
    ]);

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "issue_comment".into()), resp);
}

#[tokio::test]
async fn test_pull_request_comment() {
    let mut test = new_test();
    test.mock_pull_request_reviews(vec![]);
    test.handler.event = "pull_request_review_comment".into();
    test.handler.action = "created".into();
    test.handler.data.pull_request = some_pr();
    test.handler.data.comment = Some(Comment {
        id: 0,
        pull_request_review_id: None,
        commit_id: Some("abcdef00001111".into()),
        path: Some("src/main.rs".into()),
        body: Some("I think this file should change, cc: @mentioned-participant".into()),
        html_url: "http://the-comment".into(),
        user: User::new("joe-reviewer"),
    });
    test.handler.data.sender = User::new("joe-reviewer");
    test.mock_pull_request_commits();

    let attach = vec![
        SlackAttachmentBuilder::new("I think this file should change, cc: @mentioned-participant")
            .title("joe.reviewer said:")
            .title_link("http://the-comment")
            .build(),
    ];
    let msg = "Comment on \"<http://the-pr|The PR>\"";

    test.slack.expect(vec![
        slack::req(
            SlackRecipient::by_name("the-reviews-channel"),
            &format!("{} {}", msg, REPO_MSG),
            &attach,
            Threading::ReplyOrStart(
                "some-user/some-repo/32".to_string(),
                thread_root(REPO_MSG, 32),
            ),
        ),
        slack::req(
            SlackRecipient::user_mention("assign1"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("bob.author"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("mentioned.participant"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("the.pr.owner"),
            msg,
            &attach,
            Threading::Flat,
        ),
    ]);

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "pr_review_comment".into()), resp);
}

#[tokio::test]
async fn test_issue_comment_on_pull_request() {
    let mut test = new_test();
    test.handler.event = "issue_comment".into();
    test.handler.action = "created".into();
    test.handler.data.issue = Some(Issue {
        number: 32,
        html_url: "http://the-github-host/some-user/some-repo/pull/32".into(),
        title: "The PR".into(),
        user: User::new("the-pr-owner"),
        assignees: vec![],
    });
    test.handler.data.comment = Some(Comment {
        id: 0,
        pull_request_review_id: None,
        commit_id: None,
        path: None,
        body: Some("Looks close".into()),
        html_url: "http://the-comment".into(),
        user: User::new("joe-reviewer"),
    });
    test.handler.data.sender = User::new("joe-reviewer");
    test.github
        .get_pull_request("some-user", "some-repo", 32, Ok(some_pr().unwrap()));
    test.mock_pull_request_reviews(vec![Review::new("lgtm", User::new("smith-reviewer"))]);
    test.mock_pull_request_commits();

    let attach = vec![
        SlackAttachmentBuilder::new("Looks close")
            .title("joe.reviewer said:")
            .title_link("http://the-comment")
            .build(),
    ];
    let msg = "Comment on \"<http://the-pr|The PR>\"";

    test.slack.expect(vec![
        slack::req(
            SlackRecipient::by_name("the-reviews-channel"),
            &format!("{} {}", msg, REPO_MSG),
            &attach,
            Threading::ReplyOrStart(
                "some-user/some-repo/32".to_string(),
                thread_root(REPO_MSG, 32),
            ),
        ),
        slack::req(
            SlackRecipient::user_mention("assign1"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("bob.author"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("smith.reviewer"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("the.pr.owner"),
            msg,
            &attach,
            Threading::Flat,
        ),
    ]);

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "issue_comment".into()), resp);
    // The lookup feeds the people cache too.
    assert!(test.handler.people_cache.get(&the_repo(), 32).is_some());
}

#[tokio::test]
async fn test_issue_comment_on_pull_request_lookup_fails() {
    let mut test = new_test();
    test.handler.event = "issue_comment".into();
    test.handler.action = "created".into();
    test.handler.data.issue = Some(Issue {
        number: 32,
        html_url: "http://the-github-host/some-user/some-repo/pull/32".into(),
        title: "The PR".into(),
        user: User::new("the-pr-owner"),
        assignees: vec![User::new("assign1")],
    });
    test.handler.data.comment = Some(Comment {
        id: 0,
        pull_request_review_id: None,
        commit_id: None,
        path: None,
        body: Some("Looks close".into()),
        html_url: "http://the-comment".into(),
        user: User::new("joe-reviewer"),
    });
    test.handler.data.sender = User::new("joe-reviewer");
    test.github
        .get_pull_request("some-user", "some-repo", 32, Err(anyhow!("nope")));

    let attach = vec![
        SlackAttachmentBuilder::new("Looks close")
            .title("joe.reviewer said:")
            .title_link("http://the-comment")
            .build(),
    ];
    let msg = "Comment on \"<http://the-github-host/some-user/some-repo/pull/32|The PR>\"";

    // Notified as a plain issue: no branch, commits or reviewers.
    test.slack.expect(vec![
        slack::req(
            SlackRecipient::by_name("the-reviews-channel"),
            &format!("{} {}", msg, REPO_MSG),
            &attach,
            Threading::Reply("some-user/some-repo/32".to_string()),
        ),
        slack::req(
            SlackRecipient::user_mention("assign1"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("the.pr.owner"),
            msg,
            &attach,
            Threading::Flat,
        ),
    ]);

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "issue_comment".into()), resp);
}

#[tokio::test]
async fn test_issue_comment_on_pull_request_ignores_octobot() {
    let mut test = new_test();
    test.handler.event = "issue_comment".into();
    test.handler.action = "created".into();
    test.handler.data.issue = Some(Issue {
        number: 32,
        html_url: "http://the-github-host/some-user/some-repo/pull/32".into(),
        title: "The PR".into(),
        user: User::new("the-pr-owner"),
        assignees: vec![],
    });
    test.handler.data.comment = Some(Comment {
        id: 0,
        pull_request_review_id: None,
        commit_id: None,
        path: None,
        body: Some("Build passed".into()),
        html_url: "http://the-comment".into(),
        user: User::new("octobot[bot]"),
    });
    test.handler.data.sender = User::new("octobot[bot]");

    // No lookups at all: not the PR, its reviewers, or its commits.
    test.slack.expect(vec![]);

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "issue_comment".into()), resp);
}

#[tokio::test(start_paused = true)]
async fn test_pull_request_review_comment_grouped() {
    let mut test = new_test();
    test.mock_pull_request_reviews(vec![]);
    test.handler.event = "pull_request_review_comment".into();
    test.handler.action = "created".into();
    test.handler.data.pull_request = some_pr();
    test.handler.data.comment = Some(Comment {
        id: 0,
        pull_request_review_id: Some(77),
        commit_id: Some("abcdef00001111".into()),
        path: Some("src/main.rs".into()),
        body: Some("I think this file should change".into()),
        html_url: "http://the-comment".into(),
        user: User::new("joe-reviewer"),
    });
    test.handler.data.sender = User::new("joe-reviewer");
    test.mock_pull_request_commits();

    let mut assign1 = test.config.users().lookup_info("assign1").unwrap();
    assign1.group_review_direct_messages = Some(true);
    test.config.users_write().update(&assign1).unwrap();

    let grouped = Arc::new(CaptureSlack::default());
    test.handler.messenger = messenger::new(test.config.clone(), test.slack.new_sender())
        .with_review_groups(ReviewGroups::new(
            grouped.clone(),
            Duration::from_millis(20),
            Duration::from_secs(1),
        ));

    let attach = vec![
        SlackAttachmentBuilder::new("I think this file should change")
            .title("joe.reviewer said:")
            .title_link("http://the-comment")
            .build(),
    ];
    let msg = "Comment on \"<http://the-pr|The PR>\"";

    test.slack.expect(vec![
        slack::req(
            SlackRecipient::by_name("the-reviews-channel"),
            &format!("{} {}", msg, REPO_MSG),
            &attach,
            Threading::ReplyOrStart(
                "some-user/some-repo/32".to_string(),
                thread_root(REPO_MSG, 32),
            ),
        ),
        slack::req(
            SlackRecipient::user_mention("bob.author"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("the.pr.owner"),
            msg,
            &attach,
            Threading::Flat,
        ),
    ]);

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "pr_review_comment".into()), resp);
    assert!(grouped.reqs.lock().unwrap().is_empty());

    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        vec![slack::req(
            SlackRecipient::user_mention("assign1"),
            "joe.reviewer reviewed PR \"<http://the-pr|The PR>\"",
            &[
                SlackAttachmentBuilder::new("I think this file should change")
                    .title("src/main.rs")
                    .title_link("http://the-comment")
                    .build(),
            ],
            Threading::Flat,
        )],
        *grouped.reqs.lock().unwrap()
    );
}

fn ungrouped_review_reqs(msg: &str, attach: &[slack::SlackAttachment]) -> Vec<slack::SlackRequest> {
    vec![
        slack::req(
            SlackRecipient::by_name("the-reviews-channel"),
            &format!("{} {}", msg, REPO_MSG),
            attach,
            Threading::ReplyOrStart(
                "some-user/some-repo/32".to_string(),
                thread_root(REPO_MSG, 32),
            ),
        ),
        slack::req(
            SlackRecipient::user_mention("bob.author"),
            msg,
            attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("the.pr.owner"),
            msg,
            attach,
            Threading::Flat,
        ),
    ]
}

#[tokio::test(start_paused = true)]
async fn test_pull_request_review_grouped_with_verdict_in_thread() {
    let mut test = new_test();
    test.handler.data.pull_request = some_pr();
    test.handler.data.sender = User::new("joe-reviewer");

    let mut assign1 = test.config.users().lookup_info("assign1").unwrap();
    assign1.group_review_direct_messages = Some(true);
    assign1.thread_direct_messages = Some(true);
    test.config.users_write().update(&assign1).unwrap();

    let grouped = Arc::new(CaptureSlack::default());
    test.handler.messenger = messenger::new(test.config.clone(), test.slack.new_sender())
        .with_review_groups(ReviewGroups::new(
            grouped.clone(),
            Duration::from_millis(20),
            Duration::from_secs(1),
        ));

    let comment_msg = "Comment on \"<http://the-pr|The PR>\"";
    test.handler.event = "pull_request_review_comment".into();
    test.handler.action = "created".into();
    // Every later event reuses the first one's reviewers.
    test.mock_pull_request_reviews(vec![]);
    for (id, path, body) in [(9, "src/b.rs", "second"), (3, "src/a.rs", "first")] {
        test.handler.data.comment = Some(Comment {
            id,
            pull_request_review_id: Some(77),
            commit_id: Some("abcdef00001111".into()),
            path: Some(path.into()),
            body: Some(body.into()),
            html_url: format!("http://comment/{}", id),
            user: User::new("joe-reviewer"),
        });
        test.mock_pull_request_commits();
        let attach = vec![
            SlackAttachmentBuilder::new(body)
                .title("joe.reviewer said:")
                .title_link(format!("http://comment/{}", id))
                .build(),
        ];
        test.slack
            .expect(ungrouped_review_reqs(comment_msg, &attach));

        let resp = test.handler.handle_event().await.unwrap();
        assert_eq!((StatusCode::OK, "pr_review_comment".into()), resp);
    }

    test.handler.event = "pull_request_review".into();
    test.handler.action = "submitted".into();
    test.handler.data.comment = None;
    test.handler.data.review = Some(Review {
        id: 77,
        state: "changes_requested".into(),
        body: Some("It needs some work!".into()),
        html_url: "http://the-review".into(),
        user: User::new("joe-reviewer"),
        commit_id: None,
    });
    test.mock_pull_request_commits();
    let verdict = SlackAttachmentBuilder::new("It needs some work!")
        .title("Review: Changes Requested")
        .title_link("http://the-review")
        .color("danger")
        .build();
    let verdict_msg = "joe.reviewer requested changes to PR \"<http://the-pr|The PR>\"";
    test.slack.expect(ungrouped_review_reqs(
        verdict_msg,
        std::slice::from_ref(&verdict),
    ));

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "pr_review".into()), resp);
    assert!(grouped.reqs.lock().unwrap().is_empty());

    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        vec![slack::req(
            SlackRecipient::user_mention("assign1"),
            verdict_msg,
            &[
                verdict,
                SlackAttachmentBuilder::new("first")
                    .title("src/a.rs")
                    .title_link("http://comment/3")
                    .build(),
                SlackAttachmentBuilder::new("second")
                    .title("src/b.rs")
                    .title_link("http://comment/9")
                    .build(),
            ],
            Threading::ReplyOrStart("some-user/some-repo/32".into(), dm_thread_root(32)),
        )],
        *grouped.reqs.lock().unwrap()
    );
}

#[derive(Default)]
struct CaptureSlack {
    reqs: std::sync::Mutex<Vec<slack::SlackRequest>>,
}

impl octobot_ops::worker::Worker<slack::SlackRequest> for CaptureSlack {
    fn send(&self, req: slack::SlackRequest) {
        self.reqs.lock().unwrap().push(req);
    }
}

// A review of only inline comments arrives as "commented" with no body.
#[tokio::test]
async fn test_pull_request_review_commented_without_body() {
    let mut test = new_test();
    test.handler.event = "pull_request_review".into();
    test.handler.action = "submitted".into();
    test.handler.data.pull_request = some_pr();
    test.handler.data.review = Some(Review {
        id: 77,
        state: "commented".into(),
        body: Some("".into()),
        html_url: "http://the-comment".into(),
        user: User::new("joe-reviewer"),
        commit_id: None,
    });
    test.handler.data.sender = User::new("joe-reviewer");

    test.slack.expect(vec![]);

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "pr_review [comment]".into()), resp);
}

#[tokio::test]
async fn test_pull_request_review_dismissed() {
    let mut test = new_test();
    test.mock_pull_request_reviews(vec![]);
    test.handler.event = "pull_request_review".into();
    test.handler.action = "submitted".into();
    test.handler.data.pull_request = some_pr();
    test.handler.data.review = Some(Review {
        id: 77,
        state: "dismissed".into(),
        body: Some("Not needed".into()),
        html_url: "http://the-review".into(),
        user: User::new("joe-reviewer"),
        commit_id: None,
    });
    test.handler.data.sender = User::new("joe-reviewer");
    // No Slack message, but its hint reaches the participants.
    test.mock_pull_request_commits();

    test.slack.expect(vec![]);

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "pr_review [ignored]".into()), resp);
}

#[tokio::test]
async fn test_pull_request_review_commented() {
    let mut test = new_test();
    test.mock_pull_request_reviews(vec![]);
    test.handler.event = "pull_request_review".into();
    test.handler.action = "submitted".into();
    test.handler.data.pull_request = some_pr();
    test.handler.data.review = Some(Review {
        id: 77,
        state: "commented".into(),
        body: Some("I think this file should change, cc: @mentioned-participant".into()),
        html_url: "http://the-comment".into(),
        user: User::new("joe-reviewer"),
        commit_id: None,
    });
    test.handler.data.sender = User::new("joe-reviewer");
    test.mock_pull_request_commits();

    let attach = vec![
        SlackAttachmentBuilder::new("I think this file should change, cc: @mentioned-participant")
            .title("joe.reviewer said:")
            .title_link("http://the-comment")
            .build(),
    ];
    let msg = "Comment on \"<http://the-pr|The PR>\"";

    test.slack.expect(vec![
        slack::req(
            SlackRecipient::by_name("the-reviews-channel"),
            &format!("{} {}", msg, REPO_MSG),
            &attach,
            Threading::ReplyOrStart(
                "some-user/some-repo/32".to_string(),
                thread_root(REPO_MSG, 32),
            ),
        ),
        slack::req(
            SlackRecipient::user_mention("assign1"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("bob.author"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("mentioned.participant"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("the.pr.owner"),
            msg,
            &attach,
            Threading::Flat,
        ),
    ]);

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "pr_review [comment]".into()), resp);
}

#[tokio::test]
async fn test_pull_request_comments_ignore_empty_messages() {
    let mut test = new_test();
    test.handler.event = "pull_request_review_comment".into();
    test.handler.action = "created".into();
    test.handler.data.pull_request = some_pr();
    test.handler.data.comment = Some(Comment {
        id: 0,
        pull_request_review_id: None,
        commit_id: Some("abcdef00001111".into()),
        path: Some("src/main.rs".into()),
        body: Some("".into()),
        html_url: "http://the-comment".into(),
        user: User::new("joe-reviewer"),
    });
    test.handler.data.sender = User::new("joe-reviewer");

    test.slack.expect(vec![]);

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "pr_review_comment".into()), resp);
}

#[tokio::test]
async fn test_pull_request_comments_ignore_octobot() {
    let mut test = new_test();
    test.handler.event = "pull_request_review_comment".into();
    test.handler.action = "created".into();
    test.handler.data.pull_request = some_pr();
    test.handler.data.comment = Some(Comment {
        id: 0,
        pull_request_review_id: None,
        commit_id: Some("abcdef00001111".into()),
        path: Some("src/main.rs".into()),
        body: Some("I think this file should change, cc: @mentioned-participant".into()),
        html_url: "http://the-comment".into(),
        user: User::new("octobot[bot]"),
    });
    test.handler.data.sender = User::new("joe-reviewer");

    test.slack.expect(vec![]);

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "pr_review_comment".into()), resp);
}

#[tokio::test]
async fn test_pull_request_comments_ignore_user() {
    let mut test = new_test();
    test.handler.event = "pull_request_review_comment".into();
    test.handler.action = "created".into();
    test.handler.data.pull_request = some_pr();
    test.handler.data.comment = Some(Comment {
        id: 0,
        pull_request_review_id: None,
        commit_id: Some("abcdef00001111".into()),
        path: Some("src/main.rs".into()),
        body: Some("I think this file should change, cc: @mentioned-participant".into()),
        html_url: "http://the-comment".into(),
        user: User::new("ignore-me[bot]"),
    });
    test.handler.data.sender = User::new("joe-reviewer");

    test.slack.expect(vec![]);

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "pr_review_comment".into()), resp);
}

const COLLAPSED_BODY: &str =
    "Looks right\n\n<details><summary>Notes</summary>\n\ncc @mentioned-participant\n\n</details>";
const COLLAPSED_TEXT: &str = "Looks right\n\n▸ Notes";

fn mentioned_reqs(msg: &str, attach: &[slack::SlackAttachment]) -> Vec<slack::SlackRequest> {
    vec![
        slack::req(
            SlackRecipient::by_name("the-reviews-channel"),
            &format!("{} {}", msg, REPO_MSG),
            attach,
            Threading::ReplyOrStart(
                "some-user/some-repo/32".to_string(),
                thread_root(REPO_MSG, 32),
            ),
        ),
        slack::req(
            SlackRecipient::user_mention("assign1"),
            msg,
            attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("bob.author"),
            msg,
            attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("mentioned.participant"),
            msg,
            attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("the.pr.owner"),
            msg,
            attach,
            Threading::Flat,
        ),
    ]
}

#[tokio::test]
async fn test_pull_request_comment_hides_collapsed_details() {
    let mut test = new_test();
    test.mock_pull_request_reviews(vec![]);
    test.handler.event = "pull_request_review_comment".into();
    test.handler.action = "created".into();
    test.handler.data.pull_request = some_pr();
    test.handler.data.comment = Some(Comment {
        id: 0,
        pull_request_review_id: None,
        commit_id: Some("abcdef00001111".into()),
        path: Some("src/main.rs".into()),
        body: Some(COLLAPSED_BODY.into()),
        html_url: "http://the-comment".into(),
        user: User::new("joe-reviewer"),
    });
    test.handler.data.sender = User::new("joe-reviewer");
    test.mock_pull_request_commits();

    let attach = vec![
        SlackAttachmentBuilder::new(COLLAPSED_TEXT)
            .title("joe.reviewer said:")
            .title_link("http://the-comment")
            .build(),
    ];
    // Mentions inside collapsed blocks still notify, as on GitHub.
    test.slack.expect(mentioned_reqs(
        "Comment on \"<http://the-pr|The PR>\"",
        &attach,
    ));

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "pr_review_comment".into()), resp);
}

#[tokio::test]
async fn test_pull_request_comments_ignore_hidden_only_messages() {
    let mut test = new_test();
    test.handler.event = "pull_request_review_comment".into();
    test.handler.action = "created".into();
    test.handler.data.pull_request = some_pr();
    test.handler.data.comment = Some(Comment {
        id: 0,
        pull_request_review_id: None,
        commit_id: Some("abcdef00001111".into()),
        path: Some("src/main.rs".into()),
        body: Some("<!-- marker -->".into()),
        html_url: "http://the-comment".into(),
        user: User::new("joe-reviewer"),
    });
    test.handler.data.sender = User::new("joe-reviewer");

    test.slack.expect(vec![]);

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "pr_review_comment".into()), resp);
}

#[tokio::test(start_paused = true)]
async fn test_pull_request_review_comment_grouped_hides_collapsed_details() {
    let mut test = new_test();
    test.mock_pull_request_reviews(vec![]);
    test.handler.event = "pull_request_review_comment".into();
    test.handler.action = "created".into();
    test.handler.data.pull_request = some_pr();
    test.handler.data.comment = Some(Comment {
        id: 0,
        pull_request_review_id: Some(77),
        commit_id: Some("abcdef00001111".into()),
        path: Some("src/main.rs".into()),
        body: Some("Looks right\n\n<details><summary>Notes</summary>\n\nmore\n\n</details>".into()),
        html_url: "http://the-comment".into(),
        user: User::new("joe-reviewer"),
    });
    test.handler.data.sender = User::new("joe-reviewer");
    test.mock_pull_request_commits();

    let mut assign1 = test.config.users().lookup_info("assign1").unwrap();
    assign1.group_review_direct_messages = Some(true);
    test.config.users_write().update(&assign1).unwrap();

    let grouped = Arc::new(CaptureSlack::default());
    test.handler.messenger = messenger::new(test.config.clone(), test.slack.new_sender())
        .with_review_groups(ReviewGroups::new(
            grouped.clone(),
            Duration::from_millis(20),
            Duration::from_secs(1),
        ));

    let attach = vec![
        SlackAttachmentBuilder::new(COLLAPSED_TEXT)
            .title("joe.reviewer said:")
            .title_link("http://the-comment")
            .build(),
    ];
    test.slack.expect(ungrouped_review_reqs(
        "Comment on \"<http://the-pr|The PR>\"",
        &attach,
    ));

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "pr_review_comment".into()), resp);

    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        vec![slack::req(
            SlackRecipient::user_mention("assign1"),
            "joe.reviewer reviewed PR \"<http://the-pr|The PR>\"",
            &[SlackAttachmentBuilder::new(COLLAPSED_TEXT)
                .title("src/main.rs")
                .title_link("http://the-comment")
                .build(),],
            Threading::Flat,
        )],
        *grouped.reqs.lock().unwrap()
    );
}

#[tokio::test]
async fn test_pull_request_review_approved_hides_collapsed_details() {
    let mut test = new_test();
    test.mock_pull_request_reviews(vec![]);
    test.handler.event = "pull_request_review".into();
    test.handler.action = "submitted".into();
    test.handler.data.pull_request = some_pr();
    test.handler.data.review = Some(Review {
        id: 77,
        state: "approved".into(),
        body: Some(COLLAPSED_BODY.into()),
        html_url: "http://the-comment".into(),
        user: User::new("joe-reviewer"),
        commit_id: None,
    });
    test.handler.data.sender = User::new("joe-reviewer");
    test.mock_pull_request_commits();

    let attach = vec![
        SlackAttachmentBuilder::new(COLLAPSED_TEXT)
            .title("Review: Approved")
            .title_link("http://the-comment")
            .color("good")
            .build(),
    ];
    test.slack.expect(mentioned_reqs(
        "joe.reviewer approved PR \"<http://the-pr|The PR>\"",
        &attach,
    ));

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "pr_review".into()), resp);
}

#[tokio::test]
async fn test_pull_request_review_dismissed_publishes() {
    let mut test = new_test();
    test.mock_pull_request_reviews(vec![]);
    test.handler.event = "pull_request_review".into();
    test.handler.action = "dismissed".into();
    test.handler.data.pull_request = some_pr();
    test.handler.data.review = Some(Review {
        id: 77,
        state: "dismissed".into(),
        body: None,
        html_url: "http://the-comment".into(),
        user: User::new("joe-reviewer"),
        commit_id: None,
    });
    test.handler.data.sender = User::new("the-pr-owner");
    test.mock_pull_request_commits();
    let mut receiver = test.subscribe_events(Kind::PullRequestReview);

    // no slack mocks

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "pr_review".into()), resp);

    let event = receiver.try_recv().expect("published a hint");
    assert!(event.is_for("joe-reviewer"));
    assert_eq!("dismissed", event.change.action);
    assert_eq!(Some(32), event.change.number);
    assert!(receiver.try_recv().is_err());
}

#[tokio::test]
async fn test_commit_comment_hides_collapsed_details() {
    let mut test = new_test();
    test.handler.event = "commit_comment".into();
    test.handler.action = "created".into();
    test.handler.data.comment = Some(Comment {
        id: 0,
        pull_request_review_id: None,
        commit_id: Some("abcdef00001111".into()),
        path: None,
        body: Some(COLLAPSED_BODY.into()),
        html_url: "http://the-comment".into(),
        user: User::new("joe-reviewer"),
    });
    test.handler.data.sender = User::new("joe-reviewer");

    test.github.mock_get_pull_requests_by_commit(
        "some-user",
        "some-repo",
        "abcdef0",
        None,
        Ok(vec![]),
    );
    test.slack.expect(vec![slack::req(
        SlackRecipient::by_name("the-reviews-channel"),
        &format!("Comment on \"abcdef0\" (<http://the-github-host/some-user/some-repo/commit/abcdef00001111|abcdef0>) {}", REPO_MSG),
        &[SlackAttachmentBuilder::new(COLLAPSED_TEXT)
            .title("joe.reviewer said:")
            .title_link("http://the-comment")
            .build()],
        Threading::Flat,
    )]);

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "commit_comment".into()), resp);
}

#[tokio::test]
async fn test_commit_comment_ignore_hidden_only_messages() {
    let mut test = new_test();
    test.handler.event = "commit_comment".into();
    test.handler.action = "created".into();
    test.handler.data.comment = Some(Comment {
        id: 0,
        pull_request_review_id: None,
        commit_id: Some("abcdef00001111".into()),
        path: None,
        body: Some("<!-- marker -->".into()),
        html_url: "http://the-comment".into(),
        user: User::new("joe-reviewer"),
    });
    test.handler.data.sender = User::new("joe-reviewer");

    test.slack.expect(vec![]);

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "commit_comment".into()), resp);
}

#[tokio::test]
async fn test_pull_request_review_approved() {
    let mut test = new_test();
    test.mock_pull_request_reviews(vec![]);
    test.handler.event = "pull_request_review".into();
    test.handler.action = "submitted".into();
    test.handler.data.pull_request = some_pr();
    test.handler.data.review = Some(Review {
        id: 77,
        state: "approved".into(),
        body: Some("I like it! cc: @mentioned-participant".into()),
        html_url: "http://the-comment".into(),
        user: User::new("joe-reviewer"),
        commit_id: None,
    });
    test.handler.data.sender = User::new("joe-reviewer");
    test.mock_pull_request_commits();

    let attach = vec![
        SlackAttachmentBuilder::new("I like it! cc: @mentioned-participant")
            .title("Review: Approved")
            .title_link("http://the-comment")
            .color("good")
            .build(),
    ];
    let msg = "joe.reviewer approved PR \"<http://the-pr|The PR>\"";

    test.slack.expect(vec![
        slack::req(
            SlackRecipient::by_name("the-reviews-channel"),
            &format!("{} {}", msg, REPO_MSG),
            &attach,
            Threading::ReplyOrStart(
                "some-user/some-repo/32".to_string(),
                thread_root(REPO_MSG, 32),
            ),
        ),
        slack::req(
            SlackRecipient::user_mention("assign1"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("bob.author"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("mentioned.participant"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("the.pr.owner"),
            msg,
            &attach,
            Threading::Flat,
        ),
    ]);

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "pr_review".into()), resp);
}

#[tokio::test]
async fn test_pull_request_review_changes_requested() {
    let mut test = new_test();
    test.mock_pull_request_reviews(vec![]);
    test.handler.event = "pull_request_review".into();
    test.handler.action = "submitted".into();
    test.handler.data.pull_request = some_pr();
    test.handler.data.review = Some(Review {
        id: 77,
        state: "changes_requested".into(),
        body: Some("It needs some work! cc: @mentioned-participant".into()),
        html_url: "http://the-comment".into(),
        user: User::new("joe-reviewer"),
        commit_id: None,
    });
    test.handler.data.sender = User::new("joe-reviewer");
    test.mock_pull_request_commits();

    let attach = vec![
        SlackAttachmentBuilder::new("It needs some work! cc: @mentioned-participant")
            .title("Review: Changes Requested")
            .title_link("http://the-comment")
            .color("danger")
            .build(),
    ];
    let msg = "joe.reviewer requested changes to PR \"<http://the-pr|The PR>\"";
    test.slack.expect(vec![
        slack::req(
            SlackRecipient::by_name("the-reviews-channel"),
            &format!("{} {}", msg, REPO_MSG),
            &attach,
            Threading::ReplyOrStart(
                "some-user/some-repo/32".to_string(),
                thread_root(REPO_MSG, 32),
            ),
        ),
        slack::req(
            SlackRecipient::user_mention("assign1"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("bob.author"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("mentioned.participant"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("the.pr.owner"),
            msg,
            &attach,
            Threading::Flat,
        ),
    ]);

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "pr_review".into()), resp);
}

#[tokio::test]
async fn test_pull_request_opened() {
    let mut test = new_test();
    test.mock_pull_request_reviews(vec![]);
    test.handler.event = "pull_request".into();
    test.handler.action = "opened".into();
    test.handler.data.pull_request = some_pr();
    test.handler.data.sender = User::new("the-pr-owner");
    test.mock_pull_request_commits();

    expect_jira_ref_fail(&test.github);

    let attach = vec![
        SlackAttachmentBuilder::new("")
            .title("Pull Request #32: \"The PR\"")
            .title_link("http://the-pr")
            .build(),
    ];
    let msg = "Pull Request opened by the.pr.owner";

    test.slack.expect(vec![slack::req(
        SlackRecipient::by_name("the-reviews-channel"),
        &format!("{} {}", msg, REPO_MSG),
        &attach,
        Threading::Root("some-user/some-repo/32".to_string()),
    )]);

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "pr".into()), resp);
}

#[tokio::test]
async fn test_pull_request_closed() {
    let mut test = new_test();
    test.mock_pull_request_reviews(vec![]);
    test.handler.event = "pull_request".into();
    test.handler.action = "closed".into();
    test.handler.data.pull_request = some_pr();
    test.handler.data.sender = User::new("the-pr-closer");
    test.mock_pull_request_commits();

    let attach = vec![
        SlackAttachmentBuilder::new("")
            .title("Pull Request #32: \"The PR\"")
            .title_link("http://the-pr")
            .build(),
    ];
    let msg = "Pull Request closed";

    test.slack.expect(vec![
        slack::req(
            SlackRecipient::by_name("the-reviews-channel"),
            &format!("{} {}", msg, REPO_MSG),
            &attach,
            Threading::ReplyOrStart(
                "some-user/some-repo/32".to_string(),
                thread_root(REPO_MSG, 32),
            ),
        ),
        slack::req(
            SlackRecipient::user_mention("assign1"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("bob.author"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("joe.reviewer"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("the.pr.owner"),
            msg,
            &attach,
            Threading::Flat,
        ),
    ]);

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "pr".into()), resp);
}

#[tokio::test]
async fn test_pull_request_closed_draft_publishes() {
    let mut test = new_test();
    test.mock_pull_request_reviews(vec![]);
    test.handler.event = "pull_request".into();
    test.handler.action = "closed".into();
    let mut pr = some_pr().unwrap();
    pr.draft = Some(true);
    test.handler.data.pull_request = Some(pr);
    test.handler.data.sender = User::new("the-pr-closer");
    test.mock_pull_request_commits();
    let mut receiver = test.subscribe_events(Kind::PullRequest);

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "pr".into()), resp);

    let event = receiver.try_recv().expect("published a hint");
    assert_eq!(
        vec![
            "assign1",
            "bob-author",
            "joe-reviewer",
            "the-pr-closer",
            "the-pr-owner"
        ],
        event.recipients.iter().collect::<Vec<_>>()
    );
    assert_eq!(
        Change {
            repo: "some-user/some-repo".into(),
            kind: Kind::PullRequest,
            action: "closed".into(),
            number: Some(32),
        },
        event.change
    );
    assert!(receiver.try_recv().is_err());
}

#[tokio::test]
async fn test_pull_request_opened_draft_publishes() {
    let mut test = new_test();
    test.mock_pull_request_reviews(vec![]);
    test.handler.event = "pull_request".into();
    test.handler.action = "opened".into();
    let mut pr = some_pr().unwrap();
    pr.draft = Some(true);
    test.handler.data.pull_request = Some(pr);
    test.mock_pull_request_commits();
    expect_jira_ref_fail(&test.github);
    let mut receiver = test.subscribe_events(Kind::PullRequest);

    // no slack mocks

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "pr".into()), resp);

    let event = receiver.try_recv().expect("published a hint");
    assert!(event.is_for("the-pr-owner"));
    assert_eq!("opened", event.change.action);
    assert_eq!(Some(32), event.change.number);
    assert!(receiver.try_recv().is_err());
}

#[tokio::test]
async fn test_pull_request_unlisted_action_publishes() {
    let mut test = new_test();
    test.mock_pull_request_reviews(vec![]);
    test.handler.event = "pull_request".into();
    test.handler.action = "milestoned".into();
    test.handler.data.pull_request = some_pr();
    test.mock_pull_request_commits();
    let mut receiver = test.subscribe_events(Kind::PullRequest);

    // no slack mocks

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "pr".into()), resp);

    let event = receiver.try_recv().expect("published a hint");
    assert!(event.is_for("the-pr-owner"));
    assert_eq!("milestoned", event.change.action);
    assert_eq!(Some(32), event.change.number);
    assert!(receiver.try_recv().is_err());
}

#[tokio::test]
async fn test_pull_request_review_request_removed_publishes_to_removed_reviewer() {
    let mut test = new_test();
    test.mock_pull_request_reviews(vec![]);
    test.handler.event = "pull_request".into();
    test.handler.action = "review_request_removed".into();
    test.handler.data.pull_request = some_pr();
    test.handler.data.requested_reviewer = Some(User::new("smith-reviewer"));
    test.mock_pull_request_commits();
    let mut receiver = test.subscribe_events(Kind::PullRequest);

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "pr".into()), resp);

    let event = receiver.try_recv().expect("published a hint");
    assert!(event.is_for("smith-reviewer"));
    assert!(event.is_for("the-pr-owner"));
    assert!(receiver.try_recv().is_err());
}

#[tokio::test]
async fn test_pull_request_synchronize_leaves_hint_to_push() {
    let mut test = new_test();
    test.handler.event = "pull_request".into();
    test.handler.action = "synchronize".into();
    test.handler.data.pull_request = some_pr();
    test.mock_pull_request_commits();
    expect_jira_ref_fail(&test.github);
    let mut receiver = test.subscribe_events(Kind::PullRequest);

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "pr".into()), resp);
    assert!(receiver.try_recv().is_err());
}

#[tokio::test]
async fn test_pull_request_converted_to_draft_publishes() {
    let mut test = new_test();
    test.mock_pull_request_reviews(vec![]);
    test.handler.event = "pull_request".into();
    test.handler.action = "converted_to_draft".into();
    let mut pr = some_pr().unwrap();
    pr.draft = Some(true);
    test.handler.data.pull_request = Some(pr);
    test.mock_pull_request_commits();
    let mut receiver = test.subscribe_events(Kind::PullRequest);

    // no slack mocks

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "pr".into()), resp);

    let event = receiver.try_recv().expect("published a hint");
    assert!(event.is_for("the-pr-owner"));
    assert_eq!("converted_to_draft", event.change.action);
    assert_eq!(Some(32), event.change.number);
    assert!(receiver.try_recv().is_err());
}

#[tokio::test]
async fn test_pull_request_hint_reaches_past_reviewers() {
    let mut test = new_test();
    test.mock_pull_request_reviews(vec![Review::new("lgtm", User::new("smith-reviewer"))]);
    test.handler.event = "pull_request".into();
    test.handler.action = "converted_to_draft".into();
    let mut pr = some_pr().unwrap();
    pr.draft = Some(true);
    test.handler.data.pull_request = Some(pr);
    test.mock_pull_request_commits();
    let mut receiver = test.subscribe_events(Kind::PullRequest);

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "pr".into()), resp);

    let event = receiver.try_recv().expect("published a hint");
    assert!(event.is_for("smith-reviewer"));
}

#[tokio::test]
async fn test_pull_request_reopened() {
    let mut test = new_test();
    test.mock_pull_request_reviews(vec![]);
    test.handler.event = "pull_request".into();
    test.handler.action = "reopened".into();
    test.handler.data.pull_request = some_pr();
    test.handler.data.sender = User::new("the-pr-closer");
    test.mock_pull_request_commits();

    let attach = vec![
        SlackAttachmentBuilder::new("")
            .title("Pull Request #32: \"The PR\"")
            .title_link("http://the-pr")
            .build(),
    ];
    let msg = "Pull Request reopened";

    test.slack.expect(vec![slack::req(
        SlackRecipient::by_name("the-reviews-channel"),
        &format!("{} {}", msg, REPO_MSG),
        &attach,
        Threading::ReplyOrStart(
            "some-user/some-repo/32".to_string(),
            thread_root(REPO_MSG, 32),
        ),
    )]);
    let mut receiver = test.subscribe_events(Kind::PullRequest);

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "pr".into()), resp);

    let event = receiver.try_recv().expect("published a hint");
    assert!(event.is_for("the-pr-owner"));
    assert_eq!("reopened", event.change.action);
    assert!(receiver.try_recv().is_err());
}

#[tokio::test]
async fn test_pull_request_ready_for_review() {
    let mut test = new_test();
    test.mock_pull_request_reviews(vec![]);
    test.handler.event = "pull_request".into();
    test.handler.action = "ready_for_review".into();
    test.handler.data.pull_request = some_pr();
    test.handler.data.sender = User::new("the-pr-closer");
    test.mock_pull_request_commits();

    expect_jira_ref_fail(&test.github);

    let attach = vec![
        SlackAttachmentBuilder::new("")
            .title("Pull Request #32: \"The PR\"")
            .title_link("http://the-pr")
            .build(),
    ];
    let msg = "Pull Request is ready for review";

    test.slack.expect(vec![
        slack::req(
            SlackRecipient::by_name("the-reviews-channel"),
            &format!("{} {}", msg, REPO_MSG),
            &attach,
            Threading::ReplyOrStart(
                "some-user/some-repo/32".to_string(),
                thread_root(REPO_MSG, 32),
            ),
        ),
        slack::req(
            SlackRecipient::user_mention("assign1"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("bob.author"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("joe.reviewer"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("the.pr.owner"),
            msg,
            &attach,
            Threading::Flat,
        ),
    ]);

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "pr".into()), resp);
}

#[tokio::test]
async fn test_pull_request_ready_for_review_notifies_past_reviewers() {
    let mut test = new_test();
    test.handler.event = "pull_request".into();
    test.handler.action = "ready_for_review".into();
    test.handler.data.pull_request = some_pr();
    test.handler.data.sender = User::new("the-pr-closer");
    test.mock_pull_request_commits();
    test.mock_pull_request_reviews(vec![Review::new("lgtm", User::new("smith-reviewer"))]);

    expect_jira_ref_fail(&test.github);

    let attach = vec![
        SlackAttachmentBuilder::new("")
            .title("Pull Request #32: \"The PR\"")
            .title_link("http://the-pr")
            .build(),
    ];
    let msg = "Pull Request is ready for review";

    test.slack.expect(vec![
        slack::req(
            SlackRecipient::by_name("the-reviews-channel"),
            &format!("{} {}", msg, REPO_MSG),
            &attach,
            Threading::ReplyOrStart(
                "some-user/some-repo/32".to_string(),
                thread_root(REPO_MSG, 32),
            ),
        ),
        slack::req(
            SlackRecipient::user_mention("assign1"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("bob.author"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("joe.reviewer"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("smith.reviewer"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("the.pr.owner"),
            msg,
            &attach,
            Threading::Flat,
        ),
    ]);

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "pr".into()), resp);
}

#[tokio::test]
async fn test_pull_request_ready_for_review_reviewer_lookup_fails() {
    let mut test = new_test();
    test.handler.event = "pull_request".into();
    test.handler.action = "ready_for_review".into();
    test.handler.data.pull_request = some_pr();
    test.handler.data.sender = User::new("the-pr-closer");
    test.mock_pull_request_commits();
    test.github.mock_get_pull_request_reviews(
        "some-user",
        "some-repo",
        32,
        Err(anyhow!("reviews are down")),
    );

    expect_jira_ref_fail(&test.github);

    let attach = vec![
        SlackAttachmentBuilder::new("")
            .title("Pull Request #32: \"The PR\"")
            .title_link("http://the-pr")
            .build(),
    ];
    let msg = "Pull Request is ready for review";

    // Everyone the webhook names is still notified.
    test.slack.expect(vec![
        slack::req(
            SlackRecipient::by_name("the-reviews-channel"),
            &format!("{} {}", msg, REPO_MSG),
            &attach,
            Threading::ReplyOrStart(
                "some-user/some-repo/32".to_string(),
                thread_root(REPO_MSG, 32),
            ),
        ),
        slack::req(
            SlackRecipient::user_mention("assign1"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("bob.author"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("joe.reviewer"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("the.pr.owner"),
            msg,
            &attach,
            Threading::Flat,
        ),
    ]);

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "pr".into()), resp);
    assert_eq!(
        1,
        test.handler.metrics.github_reviewer_lookup_failures.get()
    );
}

#[tokio::test]
async fn test_pull_request_edited() {
    let mut test = new_test();
    test.mock_pull_request_reviews(vec![]);
    test.handler.event = "pull_request".into();
    test.handler.action = "edited".into();
    test.handler.data.pull_request = some_pr();
    test.handler.data.sender = User::new("the-pr-closer");
    test.mock_pull_request_commits();

    expect_jira_ref_fail(&test.github);
    let mut receiver = test.subscribe_events(Kind::PullRequest);

    // no slack mocks

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "pr".into()), resp);

    let event = receiver.try_recv().expect("published a hint");
    assert!(event.is_for("the-pr-closer"));
    assert_eq!("edited", event.change.action);
    assert!(receiver.try_recv().is_err());
}

#[tokio::test]
async fn test_pull_request_synchronize() {
    let mut test = new_test();
    test.handler.event = "pull_request".into();
    test.handler.action = "synchronize".into();
    test.handler.data.pull_request = some_pr();
    test.handler.data.sender = User::new("the-pr-closer");
    test.mock_pull_request_commits();

    expect_jira_ref_fail(&test.github);

    // no slack mocks

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "pr".into()), resp);
}

#[tokio::test]
async fn test_pull_request_assigned() {
    let mut test = new_test();
    test.mock_pull_request_reviews(vec![]);
    test.handler.event = "pull_request".into();
    test.handler.action = "assigned".into();
    test.handler.data.pull_request = some_pr();
    if let Some(ref mut pr) = test.handler.data.pull_request {
        pr.assignees.push(User::new("assign2"));
    }
    test.handler.data.sender = User::new("the-pr-closer");
    test.mock_pull_request_commits();

    let attach = vec![
        SlackAttachmentBuilder::new("")
            .title("Pull Request #32: \"The PR\"")
            .title_link("http://the-pr")
            .build(),
    ];
    let msg = "Pull Request assigned to assign1, assign2";

    test.slack.expect(vec![
        slack::req(
            SlackRecipient::by_name("the-reviews-channel"),
            &format!("{} {}", msg, REPO_MSG),
            &attach,
            Threading::ReplyOrStart(
                "some-user/some-repo/32".to_string(),
                thread_root(REPO_MSG, 32),
            ),
        ),
        slack::req(
            SlackRecipient::user_mention("assign1"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("assign2"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("bob.author"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("joe.reviewer"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("the.pr.owner"),
            msg,
            &attach,
            Threading::Flat,
        ),
    ]);

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "pr".into()), resp);
}

#[tokio::test]
async fn test_pull_request_unassigned() {
    let mut test = new_test();
    test.mock_pull_request_reviews(vec![]);
    test.handler.event = "pull_request".into();
    test.handler.action = "unassigned".into();
    test.handler.data.pull_request = some_pr();
    test.handler.data.sender = User::new("the-pr-closer");
    test.mock_pull_request_commits();

    let attach = vec![
        SlackAttachmentBuilder::new("")
            .title("Pull Request #32: \"The PR\"")
            .title_link("http://the-pr")
            .build(),
    ];
    let msg = "Pull Request unassigned";

    test.slack.expect(vec![slack::req(
        SlackRecipient::by_name("the-reviews-channel"),
        &format!("{} {}", msg, REPO_MSG),
        &attach,
        Threading::ReplyOrStart(
            "some-user/some-repo/32".to_string(),
            thread_root(REPO_MSG, 32),
        ),
    )]);

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "pr".into()), resp);
}

#[tokio::test]
async fn test_pull_request_review_requested() {
    let mut test = new_test();
    test.mock_pull_request_reviews(vec![]);
    test.handler.event = "pull_request".into();
    test.handler.action = "review_requested".into();
    test.handler.data.pull_request = some_pr();
    if let Some(ref mut pr) = test.handler.data.pull_request {
        pr.requested_reviewers = Some(vec![User::new("joe-reviewer"), User::new("smith-reviewer")]);
        pr.requested_teams = Some(vec![Team::new(100, "team-awesome")]);
    }
    test.handler.data.sender = User::new("the-pr-closer");
    test.mock_pull_request_commits();
    test.mock_get_team_members(100);

    let attach = vec![
        SlackAttachmentBuilder::new("")
            .title("Pull Request #32: \"The PR\"")
            .title_link("http://the-pr")
            .build(),
    ];
    let msg = "Pull Request by the.pr.owner submitted for review to joe.reviewer, smith.reviewer, @team-awesome";

    test.slack.expect(vec![
        slack::req(
            SlackRecipient::by_name("the-reviews-channel"),
            &format!("{} {}", msg, REPO_MSG),
            &attach,
            Threading::ReplyOrStart(
                "some-user/some-repo/32".to_string(),
                thread_root(REPO_MSG, 32),
            ),
        ),
        slack::req(
            SlackRecipient::user_mention("assign1"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("bob.author"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("joe.reviewer"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("smith.reviewer"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("team.member1"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("team.member2"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("the.pr.owner"),
            msg,
            &attach,
            Threading::Flat,
        ),
    ]);

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "pr".into()), resp);
}

#[tokio::test]
async fn test_pull_request_review_requested_muted_team() {
    let mut test = new_test();
    test.mock_pull_request_reviews(vec![]);
    test.handler.event = "pull_request".into();
    test.handler.action = "review_requested".into();
    test.handler.data.pull_request = some_pr();
    if let Some(ref mut pr) = test.handler.data.pull_request {
        pr.requested_reviewers = Some(vec![User::new("joe-reviewer"), User::new("smith-reviewer")]);
        pr.requested_teams = Some(vec![Team::new(100, "team-awesome")]);
    }
    test.handler.data.sender = User::new("the-pr-closer");
    test.mock_pull_request_commits();
    test.mock_get_team_members(100);

    let mut member = test.config.users().lookup_info("team-member1").unwrap();
    member.muted_teams = vec!["some-user/team-awesome".into()];
    test.config.users_write().update(&member).unwrap();

    let attach = vec![
        SlackAttachmentBuilder::new("")
            .title("Pull Request #32: \"The PR\"")
            .title_link("http://the-pr")
            .build(),
    ];
    let msg = "Pull Request by the.pr.owner submitted for review to joe.reviewer, smith.reviewer, @team-awesome";

    test.slack.expect(vec![
        slack::req(
            SlackRecipient::by_name("the-reviews-channel"),
            &format!("{} {}", msg, REPO_MSG),
            &attach,
            Threading::ReplyOrStart(
                "some-user/some-repo/32".to_string(),
                thread_root(REPO_MSG, 32),
            ),
        ),
        slack::req(
            SlackRecipient::user_mention("assign1"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("bob.author"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("joe.reviewer"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("smith.reviewer"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("team.member2"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("the.pr.owner"),
            msg,
            &attach,
            Threading::Flat,
        ),
    ]);

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "pr".into()), resp);
}

#[tokio::test]
async fn test_pull_request_review_no_username() {
    let mut test = new_test();
    test.mock_pull_request_reviews(vec![]);
    test.handler.event = "pull_request".into();
    test.handler.action = "review_requested".into();
    test.handler.data.pull_request = some_pr();
    if let Some(ref mut pr) = test.handler.data.pull_request {
        pr.requested_reviewers = Some(vec![User::new("some-unknown-reviewer")]);
        pr.requested_teams = Some(vec![Team::new(100, "some-unknown-team")]);
    }
    test.handler.data.sender = User::new("the-pr-closer");
    test.mock_pull_request_commits();
    test.github
        .mock_get_team_members(&the_repo(), 100, Err(anyhow!("nope")));

    let attach = vec![
        SlackAttachmentBuilder::new("")
            .title("Pull Request #32: \"The PR\"")
            .title_link("http://the-pr")
            .build(),
    ];
    let msg = "Pull Request by the.pr.owner submitted for review to some-unknown-reviewer, @some-unknown-team";

    test.slack.expect(vec![
        slack::req(
            SlackRecipient::by_name("the-reviews-channel"),
            &format!("{} {}", msg, REPO_MSG),
            &attach,
            Threading::ReplyOrStart(
                "some-user/some-repo/32".to_string(),
                thread_root(REPO_MSG, 32),
            ),
        ),
        slack::req(
            SlackRecipient::user_mention("assign1"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("bob.author"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("the.pr.owner"),
            msg,
            &attach,
            Threading::Flat,
        ),
    ]);

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "pr".into()), resp);
}

#[tokio::test]
async fn test_pull_request_labeled_not_merged() {
    let mut test = new_test();
    test.mock_pull_request_reviews(vec![]);
    test.handler.event = "pull_request".into();
    test.handler.action = "labeled".into();
    test.handler.data.pull_request = some_pr();
    if let Some(ref mut pr) = test.handler.data.pull_request {
        pr.merged = Some(false);
    }
    test.handler.data.sender = User::new("the-pr-owner");
    test.mock_pull_request_commits();

    // labeled but not merged --> noop

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "pr".into()), resp);
}

#[tokio::test]
async fn test_pull_request_merged_error_getting_labels() {
    let mut test = new_test();
    test.mock_pull_request_reviews(vec![]);
    test.handler.event = "pull_request".into();
    test.handler.action = "closed".into();
    test.handler.data.pull_request = some_pr();
    if let Some(ref mut pr) = test.handler.data.pull_request {
        pr.merged = Some(true);
    }
    test.handler.data.sender = User::new("the-pr-merger");

    test.mock_pull_request_commits();
    test.github.mock_get_pull_request_labels(
        "some-user",
        "some-repo",
        32,
        Err(anyhow!("whooops.")),
    );

    let msg1 = "Pull Request merged";
    let attach1 = vec![
        SlackAttachmentBuilder::new("")
            .title("Pull Request #32: \"The PR\"")
            .title_link("http://the-pr")
            .build(),
    ];

    let msg2 = "Error getting Pull Request labels";
    let attach2 = vec![
        SlackAttachmentBuilder::new("whooops.")
            .color("danger")
            .build(),
    ];

    test.slack.expect(vec![
        slack::req(
            SlackRecipient::by_name("the-reviews-channel"),
            &format!("{} {}", msg1, REPO_MSG),
            &attach1,
            Threading::ReplyOrStart(
                "some-user/some-repo/32".to_string(),
                thread_root(REPO_MSG, 32),
            ),
        ),
        slack::req(
            SlackRecipient::user_mention("assign1"),
            msg1,
            &attach1,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("bob.author"),
            msg1,
            &attach1,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("joe.reviewer"),
            msg1,
            &attach1,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("the.pr.owner"),
            msg1,
            &attach1,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::by_name("the-reviews-channel"),
            &format!("{} {}", msg2, REPO_MSG),
            &attach2,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("the.pr.owner"),
            msg2,
            &attach2,
            Threading::Flat,
        ),
    ]);

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "pr".into()), resp);
}

#[tokio::test]
async fn test_pull_request_merged_no_labels() {
    let mut test = new_test();
    test.mock_pull_request_reviews(vec![]);
    test.handler.event = "pull_request".into();
    test.handler.action = "closed".into();
    test.handler.data.pull_request = some_pr();
    if let Some(ref mut pr) = test.handler.data.pull_request {
        pr.merged = Some(true);
    }
    test.handler.data.sender = User::new("the-pr-merger");

    test.mock_pull_request_commits();
    test.github
        .mock_get_pull_request_labels("some-user", "some-repo", 32, Ok(vec![]));

    let attach = vec![
        SlackAttachmentBuilder::new("")
            .title("Pull Request #32: \"The PR\"")
            .title_link("http://the-pr")
            .build(),
    ];
    let msg = "Pull Request merged";

    test.slack.expect(vec![
        slack::req(
            SlackRecipient::by_name("the-reviews-channel"),
            &format!("{} {}", msg, REPO_MSG),
            &attach,
            Threading::ReplyOrStart(
                "some-user/some-repo/32".to_string(),
                thread_root(REPO_MSG, 32),
            ),
        ),
        slack::req(
            SlackRecipient::user_mention("assign1"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("bob.author"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("joe.reviewer"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("the.pr.owner"),
            msg,
            &attach,
            Threading::Flat,
        ),
    ]);

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "pr".into()), resp);
}

#[tokio::test]
async fn test_pull_request_merged_backport_labels() {
    let mut test = new_test();
    test.mock_pull_request_reviews(vec![]);
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

    let commits = test.mock_pull_request_commits();
    test.github.mock_get_pull_request_labels(
        "some-user",
        "some-repo",
        32,
        Ok(vec![
            Label::new("other"),
            Label::new("backport-1.0"),
            Label::new("BACKPORT-2.0"),
            Label::new("BACKport-other-thing"),
            Label::new("non-matching"),
        ]),
    );

    test.slack.expect(vec![
        slack::req(
            SlackRecipient::by_name("the-reviews-channel"),
            &format!("{} {}", msg, REPO_MSG),
            &attach,
            Threading::ReplyOrStart(
                "some-user/some-repo/32".to_string(),
                thread_root(REPO_MSG, 32),
            ),
        ),
        slack::req(
            SlackRecipient::user_mention("assign1"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("bob.author"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("joe.reviewer"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("the.pr.owner"),
            msg,
            &attach,
            Threading::Flat,
        ),
    ]);

    test.expect_will_merge_branches(
        "release/",
        vec![
            "release/1.0".into(),
            "release/2.0".into(),
            "release/other-thing".into(),
        ],
        commits,
    );

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "pr".into()), resp);
}

#[tokio::test]
async fn test_pull_request_merged_backport_labels_custom_pattern() {
    let mut test = new_test();
    test.github
        .mock_get_pull_request_reviews("some-user", "custom-branches-repo", 32, Ok(vec![]));
    test.handler.event = "pull_request".into();
    test.handler.action = "closed".into();
    test.handler.data.pull_request = some_pr();
    if let Some(ref mut pr) = test.handler.data.pull_request {
        pr.merged = Some(true);
    }
    test.handler.data.sender = User::new("the-pr-merger");

    test.config
        .repos_write()
        .insert_info(
            &repos::RepoInfo::new("some-user/custom-branches-repo", "the-reviews-channel")
                .with_release_branch_prefix("the-other-prefix-".into())
                .with_use_threads(true),
        )
        .expect("Failed to add repo");

    // change the repo to an unconfigured one
    test.handler.repository = Repo::parse(&format!(
        "http://{}/some-user/custom-branches-repo",
        test.github.github_host()
    ))
    .unwrap();

    let attach = vec![
        SlackAttachmentBuilder::new("")
            .title("Pull Request #32: \"The PR\"")
            .title_link("http://the-pr")
            .build(),
    ];
    let msg = "Pull Request merged";

    let commits = some_commits();
    test.github.mock_get_pull_request_commits(
        "some-user",
        "custom-branches-repo",
        32,
        Ok(commits.clone()),
    );

    test.github.mock_get_pull_request_labels(
        "some-user",
        "custom-branches-repo",
        32,
        Ok(vec![
            Label::new("other"),
            Label::new("backport-1.0"),
            Label::new("BACKPORT-2.0"),
            Label::new("BACKport-other-thing"),
            Label::new("non-matching"),
        ]),
    );

    let repo_msg =
        "(<http://the-github-host/some-user/custom-branches-repo|some-user/custom-branches-repo>)";

    test.slack.expect(vec![
        slack::req(
            SlackRecipient::by_name("the-reviews-channel"),
            &format!("{} {}", msg, repo_msg),
            &attach,
            Threading::ReplyOrStart(
                "some-user/custom-branches-repo/32".to_string(),
                thread_root(repo_msg, 32),
            ),
        ),
        slack::req(
            SlackRecipient::user_mention("assign1"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("bob.author"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("joe.reviewer"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("the.pr.owner"),
            msg,
            &attach,
            Threading::Flat,
        ),
    ]);

    test.expect_will_merge_branches(
        "the-other-prefix-",
        vec![
            "the-other-prefix-1.0".into(),
            "the-other-prefix-2.0".into(),
            "the-other-prefix-other-thing".into(),
        ],
        commits,
    );

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "pr".into()), resp);
}

#[tokio::test]
async fn test_pull_request_merged_retroactively_labeled() {
    let mut test = new_test();
    test.mock_pull_request_reviews(vec![]);
    test.handler.event = "pull_request".into();
    test.handler.action = "labeled".into();
    test.handler.data.pull_request = some_pr();
    if let Some(ref mut pr) = test.handler.data.pull_request {
        pr.merged = Some(true);
    }
    test.handler.data.label = Some(Label::new("backport-7.123"));
    test.handler.data.sender = User::new("the-pr-merger");

    let commits = test.mock_pull_request_commits();

    test.expect_will_merge_branches("release/", vec!["release/7.123".into()], commits);

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "pr".into()), resp);
}

#[tokio::test]
async fn test_pull_request_merged_master_branch() {
    let mut test = new_test();
    test.mock_pull_request_reviews(vec![]);
    test.handler.event = "pull_request".into();
    test.handler.action = "labeled".into();
    test.handler.data.pull_request = some_pr();
    if let Some(ref mut pr) = test.handler.data.pull_request {
        pr.merged = Some(true);
    }
    test.handler.data.label = Some(Label::new("backport-master"));
    test.handler.data.sender = User::new("the-pr-merger");

    let commits = test.mock_pull_request_commits();

    test.expect_will_merge_branches("release/", vec!["master".into()], commits);

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "pr".into()), resp);
}

#[tokio::test]
async fn test_pull_request_merged_develop_branch() {
    let mut test = new_test();
    test.mock_pull_request_reviews(vec![]);
    test.handler.event = "pull_request".into();
    test.handler.action = "labeled".into();
    test.handler.data.pull_request = some_pr();
    if let Some(ref mut pr) = test.handler.data.pull_request {
        pr.merged = Some(true);
    }
    test.handler.data.label = Some(Label::new("backport-develop"));
    test.handler.data.sender = User::new("the-pr-merger");

    let commits = test.mock_pull_request_commits();

    test.expect_will_merge_branches("release/", vec!["develop".into()], commits);

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "pr".into()), resp);
}

#[tokio::test]
async fn test_pull_request_merged_main_branch() {
    let mut test = new_test();
    test.mock_pull_request_reviews(vec![]);
    test.handler.event = "pull_request".into();
    test.handler.action = "labeled".into();
    test.handler.data.pull_request = some_pr();
    if let Some(ref mut pr) = test.handler.data.pull_request {
        pr.merged = Some(true);
    }
    test.handler.data.label = Some(Label::new("backport-main"));
    test.handler.data.sender = User::new("the-pr-merger");

    let commits = test.mock_pull_request_commits();

    test.expect_will_merge_branches("release/", vec!["main".into()], commits);

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "pr".into()), resp);
}

#[tokio::test]
async fn test_push_no_pr() {
    let mut test = new_test();
    test.handler.event = "push".into();
    test.handler.data.ref_name = Some("refs/heads/some-branch".into());
    test.handler.data.before = Some("abcdef0000".into());
    test.handler.data.after = Some("1111abcdef".into());

    test.github.mock_get_pull_requests(
        "some-user",
        "some-repo",
        Some("open"),
        Some("some-branch"),
        Ok(vec![]),
    );

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "push".into()), resp);
}

#[tokio::test]
async fn test_push_branch_name_containing_refs_heads() {
    let mut test = new_test();
    test.handler.event = "push".into();
    test.handler.data.ref_name = Some("refs/heads/feature/refs/heads/fix".into());
    test.handler.data.before = Some("abcdef0000".into());
    test.handler.data.after = Some("1111abcdef".into());

    test.github.mock_get_pull_requests(
        "some-user",
        "some-repo",
        Some("open"),
        Some("feature/refs/heads/fix"),
        Ok(vec![]),
    );

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "push".into()), resp);
}

#[tokio::test]
async fn test_push_with_pr() {
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

    // no jira references here: should fail
    expect_jira_ref_fail_pr(&test.github, &pr1, &some_commits());
    expect_jira_ref_fail_pr(&test.github, &pr2, &some_commits());

    test.github
        .mock_get_pull_request_commits("some-user", "some-repo", 32, Ok(some_commits()));
    test.github
        .mock_get_pull_request_commits("some-user", "some-repo", 99, Ok(some_commits()));
    test.mock_pull_request_reviews(vec![]);
    test.github
        .mock_get_pull_request_reviews("some-user", "some-repo", 99, Ok(vec![]));

    test.github.mock_get_pull_requests(
        "some-user",
        "some-repo",
        Some("open"),
        Some("some-branch"),
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
        slack::req(
            SlackRecipient::by_name("the-reviews-channel"),
            &format!("{} {}", msg, REPO_MSG),
            &attach1,
            Threading::ReplyOrStart(
                "some-user/some-repo/32".to_string(),
                thread_root(REPO_MSG, 32),
            ),
        ),
        slack::req(
            SlackRecipient::user_mention("assign1"),
            msg,
            &attach1,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("bob.author"),
            msg,
            &attach1,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("joe.reviewer"),
            msg,
            &attach1,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("the.pr.owner"),
            msg,
            &attach1,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::by_name("the-reviews-channel"),
            &format!("{} {}", msg, REPO_MSG),
            &attach2,
            Threading::ReplyOrStart(
                "some-user/some-repo/99".to_string(),
                thread_root(REPO_MSG, 99),
            ),
        ),
        slack::req(
            SlackRecipient::user_mention("assign2"),
            msg,
            &attach2,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("bob.author"),
            msg,
            &attach2,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("the.pr.owner"),
            msg,
            &attach2,
            Threading::Flat,
        ),
    ]);

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "push".into()), resp);
    // The listing feeds the people cache too.
    let people = test.handler.people_cache.get(&the_repo(), 99).unwrap();
    assert_eq!(vec![User::new("assign2")], people.assignees);
}

#[tokio::test]
async fn test_push_force_notify() {
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
        Some("open"),
        Some("some-branch"),
        Ok(vec![pr.clone()]),
    );
    // The listing lags the push, which octobot takes as the head.
    pr.head.sha = "1111abcdef".into();

    test.mock_pull_request_commits();
    test.mock_pull_request_reviews(vec![]);

    expect_jira_ref_fail_pr(&test.github, &pr, &some_commits());

    let msg = "joe.sender pushed 0 commit(s) to branch some-branch";
    let attach = vec![
        SlackAttachmentBuilder::new("")
            .title("Pull Request #32: \"The PR\"")
            .title_link("http://the-pr")
            .build(),
    ];
    test.slack.expect(vec![
        slack::req(
            SlackRecipient::by_name("the-reviews-channel"),
            &format!("{} {}", msg, REPO_MSG),
            &attach,
            Threading::ReplyOrStart(
                "some-user/some-repo/32".to_string(),
                thread_root(REPO_MSG, 32),
            ),
        ),
        slack::req(
            SlackRecipient::user_mention("assign1"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("bob.author"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("joe.reviewer"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("the.pr.owner"),
            msg,
            &attach,
            Threading::Flat,
        ),
    ]);

    test.expect_will_force_push_notify(&pr, "abcdef0000", "1111abcdef");

    let resp = test.handler.handle_event().await.expect("handled event");
    assert_eq!((StatusCode::OK, "push".into()), resp);
}

#[tokio::test]
async fn test_push_force_notify_wip() {
    let mut test = new_test();
    test.mock_pull_request_reviews(vec![]);

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
        Some("open"),
        Some("some-branch"),
        Ok(vec![pr]),
    );
    test.mock_pull_request_commits();

    // Note: no expectations here.

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "push".into()), resp);
}

#[tokio::test]
async fn test_push_draft_publishes() {
    let mut test = new_test();
    test.mock_pull_request_reviews(vec![]);

    test.handler.event = "push".into();
    test.handler.data.ref_name = Some("refs/heads/some-branch".into());
    test.handler.data.before = Some("abcdef0000".into());
    test.handler.data.after = Some("1111abcdef".into());

    let mut pr = some_pr().unwrap();
    pr.head.sha = "abcdef0000".into();
    pr.draft = Some(true);
    test.github.mock_get_pull_requests(
        "some-user",
        "some-repo",
        Some("open"),
        Some("some-branch"),
        Ok(vec![pr]),
    );
    test.mock_pull_request_commits();
    let mut receiver = test.subscribe_events(Kind::Push);

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "push".into()), resp);

    let event = receiver.try_recv().expect("published a hint");
    assert_eq!(
        vec![
            "assign1",
            "bob-author",
            "joe-reviewer",
            "joe-sender",
            "the-pr-owner"
        ],
        event.recipients.iter().collect::<Vec<_>>()
    );
    assert_eq!(
        Change {
            repo: "some-user/some-repo".into(),
            kind: Kind::Push,
            action: "".into(),
            number: Some(32),
        },
        event.change
    );
    assert!(receiver.try_recv().is_err());
}

fn some_check(conclusion: Option<&str>) -> CheckHook {
    let pr = some_pr().unwrap();
    let check_ref = |sha: &str| CheckRef {
        sha: sha.into(),
        repo: CheckRepo { id: the_repo().id },
    };
    CheckHook {
        conclusion: conclusion.map(String::from),
        pull_requests: vec![CheckPullRequest {
            number: pr.number,
            head: check_ref(&pr.head.sha),
            base: check_ref(&pr.base.sha),
        }],
    }
}

#[tokio::test]
async fn test_check_suite_publishes() {
    let mut test = new_test();
    test.handler.event = "check_suite".into();
    test.handler.action = "completed".into();
    test.handler.data.sender = User::new("github-actions[bot]");
    test.handler.data.check_suite = Some(some_check(Some("success")));
    test.github
        .get_pull_request("some-user", "some-repo", 32, Ok(some_pr().unwrap()));
    test.mock_pull_request_reviews(vec![Review::new("lgtm", User::new("smith-reviewer"))]);
    test.mock_pull_request_commits();
    let mut receiver = test.subscribe_events(Kind::CheckSuite);

    // no slack mocks

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "check_suite".into()), resp);

    let event = receiver.try_recv().expect("published a hint");
    assert_eq!(
        vec![
            "assign1",
            "bob-author",
            "github-actions[bot]",
            "joe-reviewer",
            "smith-reviewer",
            "the-pr-owner"
        ],
        event.recipients.iter().collect::<Vec<_>>()
    );
    assert_eq!(
        Change {
            repo: "some-user/some-repo".into(),
            kind: Kind::CheckSuite,
            action: "completed".into(),
            number: Some(32),
        },
        event.change
    );
    assert!(receiver.try_recv().is_err());
}

// Every job and workflow reports a check, so checks after the first must cost no API calls.
#[tokio::test]
async fn test_check_suite_reuses_cached_participants() {
    let mut test = new_test();
    test.handler.event = "check_suite".into();
    test.handler.action = "completed".into();
    let mut pr = some_pr().unwrap();
    pr.head.sha = some_commits().last().unwrap().sha.clone();
    let mut check = some_check(Some("success"));
    check.pull_requests[0].head.sha = pr.head.sha.clone();
    test.handler.data.check_suite = Some(check);
    test.github
        .get_pull_request("some-user", "some-repo", 32, Ok(pr));
    test.mock_pull_request_reviews(vec![Review::new("lgtm", User::new("smith-reviewer"))]);
    test.mock_pull_request_commits();
    let mut receiver = test.subscribe_events(Kind::CheckSuite);

    test.handler.handle_event().await.unwrap();
    let looked_up = receiver.try_recv().expect("published a hint");

    // no more github mocks

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "check_suite".into()), resp);
    let cached = receiver.try_recv().expect("published a hint");
    assert_eq!(looked_up, cached);
}

#[tokio::test]
async fn test_check_suite_without_pull_requests() {
    let mut test = new_test();
    test.handler.event = "check_suite".into();
    test.handler.action = "completed".into();
    let mut check = some_check(Some("success"));
    check.pull_requests = vec![];
    test.handler.data.check_suite = Some(check);
    let mut receiver = test.subscribe_events(Kind::CheckSuite);

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "check_suite".into()), resp);
    assert!(receiver.try_recv().is_err());
}

// A listed PR can be another repository's, such as a fork's upstream, so its number names some
// other PR here.
#[tokio::test]
async fn test_check_suite_skips_pull_request_in_another_repo() {
    let mut test = new_test();
    test.handler.event = "check_suite".into();
    test.handler.action = "completed".into();
    let mut check = some_check(Some("success"));
    check.pull_requests[0].base.repo.id = 99;
    test.handler.data.check_suite = Some(check);
    let mut receiver = test.subscribe_events(Kind::CheckSuite);

    // no github mocks

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "check_suite".into()), resp);
    assert!(receiver.try_recv().is_err());
}

// The PR API can briefly return the old head after a push, so the webhook decides which PRs
// are at the checked commit.
#[tokio::test]
async fn test_check_suite_publishes_while_pull_request_head_lags() {
    let mut test = new_test();
    test.handler.event = "check_suite".into();
    test.handler.action = "requested".into();
    let mut check = some_check(None);
    check.pull_requests[0].head.sha = "abcdef0000".into();
    test.handler.data.check_suite = Some(check);
    test.github
        .get_pull_request("some-user", "some-repo", 32, Ok(some_pr().unwrap()));
    test.mock_pull_request_reviews(vec![]);
    test.mock_pull_request_commits();
    let mut receiver = test.subscribe_events(Kind::CheckSuite);

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "check_suite".into()), resp);

    let event = receiver.try_recv().expect("published a hint");
    assert_eq!(Some(32), event.change.number);
}

#[tokio::test]
async fn test_check_run_failed_publishes() {
    let mut test = new_test();
    test.handler.event = "check_run".into();
    test.handler.action = "completed".into();
    test.handler.data.check_run = Some(some_check(Some("failure")));
    test.github
        .get_pull_request("some-user", "some-repo", 32, Ok(some_pr().unwrap()));
    test.mock_pull_request_reviews(vec![]);
    test.mock_pull_request_commits();
    let mut receiver = test.subscribe_events(Kind::CheckRun);

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "check_run".into()), resp);

    let event = receiver.try_recv().expect("published a hint");
    assert!(event.is_for("the-pr-owner"));
    assert_eq!(Kind::CheckRun, event.change.kind);
    assert_eq!("completed", event.change.action);
    assert_eq!(Some(32), event.change.number);
    assert!(receiver.try_recv().is_err());
}

#[tokio::test]
async fn test_check_run_passed_is_ignored() {
    let mut test = new_test();
    test.handler.event = "check_run".into();
    test.handler.action = "completed".into();
    test.handler.data.check_run = Some(some_check(Some("success")));
    let mut receiver = test.subscribe_events(Kind::CheckRun);

    // no github mocks: a run that is not worth a hint costs no API calls

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "check_run [ignored]".into()), resp);
    assert!(receiver.try_recv().is_err());
}

// A rerequested run still carries its old conclusion.
#[tokio::test]
async fn test_check_run_rerequested_is_ignored() {
    let mut test = new_test();
    test.handler.event = "check_run".into();
    test.handler.action = "rerequested".into();
    test.handler.data.check_run = Some(some_check(Some("failure")));
    let mut receiver = test.subscribe_events(Kind::CheckRun);

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "check_run [ignored]".into()), resp);
    assert!(receiver.try_recv().is_err());
}

#[tokio::test]
async fn test_push_force_notify_ignored() {
    let mut test = new_test();

    test.handler.event = "push".into();
    test.handler.data.ref_name = Some("refs/heads/some-branch".into());
    test.handler.data.before = Some("abcdef0000".into());
    test.handler.data.after = Some("1111abcdef".into());
    test.handler.data.forced = Some(true);

    // change the repo to an unconfigured one
    test.handler.repository = Repo::parse(&format!(
        "http://{}/some-other-user/some-other-repo",
        test.github.github_host()
    ))
    .unwrap();

    let mut pr = some_pr().unwrap();
    pr.head.sha = "1111abcdef".into();
    test.github.mock_get_pull_requests(
        "some-other-user",
        "some-other-repo",
        Some("open"),
        Some("some-branch"),
        Ok(vec![pr.clone()]),
    );
    test.github.mock_get_pull_request_commits(
        "some-other-user",
        "some-other-repo",
        32,
        Ok(some_commits()),
    );
    test.github
        .mock_get_pull_request_reviews("some-other-user", "some-other-repo", 32, Ok(vec![]));
    expect_jira_ref_fail_pr(&test.github, &pr, &some_commits());

    let msg = "joe.sender pushed 0 commit(s) to branch some-branch";
    let attach = vec![
        SlackAttachmentBuilder::new("")
            .title("Pull Request #32: \"The PR\"")
            .title_link("http://the-pr")
            .build(),
    ];
    test.slack.expect(vec![
        slack::req(
            SlackRecipient::user_mention("assign1"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("bob.author"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("joe.reviewer"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("the.pr.owner"),
            msg,
            &attach,
            Threading::Flat,
        ),
    ]);

    // Note: no expectations here.

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "push".into()), resp);
}

#[tokio::test]
async fn test_team_members_cache() {
    let mut test = new_test();
    test.mock_pull_request_reviews(vec![]);
    test.handler.event = "pull_request".into();
    test.handler.action = "review_requested".into();
    test.handler.data.pull_request = some_pr();
    if let Some(ref mut pr) = test.handler.data.pull_request {
        pr.requested_teams = Some(vec![Team::new(100, "team-awesome")]);
    }
    test.handler.data.sender = User::new("the-pr-closer");
    test.mock_pull_request_commits();
    test.mock_pull_request_commits();
    test.mock_get_team_members(100);

    let attach = vec![
        SlackAttachmentBuilder::new("")
            .title("Pull Request #32: \"The PR\"")
            .title_link("http://the-pr")
            .build(),
    ];
    let msg = "Pull Request by the.pr.owner submitted for review to joe.reviewer, @team-awesome";
    test.slack.expect(vec![
        slack::req(
            SlackRecipient::by_name("the-reviews-channel"),
            &format!("{} {}", msg, REPO_MSG),
            &attach,
            Threading::ReplyOrStart(
                "some-user/some-repo/32".to_string(),
                thread_root(REPO_MSG, 32),
            ),
        ),
        slack::req(
            SlackRecipient::user_mention("assign1"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("bob.author"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("joe.reviewer"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("team.member1"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("team.member2"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("the.pr.owner"),
            msg,
            &attach,
            Threading::Flat,
        ),
    ]);

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "pr".into()), resp);

    // Second request. get_team_members is not mocked on this request.
    test.mock_pull_request_commits();
    let attach = vec![
        SlackAttachmentBuilder::new("")
            .title("Pull Request #32: \"The PR\"")
            .title_link("http://the-pr")
            .build(),
    ];
    let msg = "Pull Request by the.pr.owner submitted for review to joe.reviewer, @team-awesome";
    test.slack.expect(vec![
        slack::req(
            SlackRecipient::by_name("the-reviews-channel"),
            &format!("{} {}", msg, REPO_MSG),
            &attach,
            Threading::ReplyOrStart(
                "some-user/some-repo/32".to_string(),
                thread_root(REPO_MSG, 32),
            ),
        ),
        slack::req(
            SlackRecipient::user_mention("assign1"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("bob.author"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("joe.reviewer"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("team.member1"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("team.member2"),
            msg,
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("the.pr.owner"),
            msg,
            &attach,
            Threading::Flat,
        ),
    ]);
    let another_resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "pr".into()), another_resp);
}

// Runs an event whose only effect is a hint, returning who it reached.
async fn hint_recipients(test: &mut GithubHandlerTest, event: &str, action: &str) -> Vec<String> {
    test.handler.event = event.into();
    test.handler.action = action.into();
    let kind = Kind::from_event(event).unwrap();
    let mut receiver = test.subscribe_events(kind);
    test.handler.handle_event().await.unwrap();
    let hint = receiver.try_recv().expect("published a hint");
    hint.recipients.iter().cloned().collect()
}

#[tokio::test]
async fn test_reviewers_cache() {
    let mut test = new_test();
    test.handler.data.pull_request = some_pr();
    test.handler.data.sender = User::new("the-pr-owner");

    test.mock_pull_request_reviews(vec![Review::new("", User::new("past-reviewer"))]);
    test.mock_pull_request_commits();
    let recipients = hint_recipients(&mut test, "pull_request", "milestoned").await;
    assert!(recipients.contains(&"past-reviewer".into()));

    // A review webhook adds its reviewer without another lookup.
    test.handler.data.review = Some(Review::new("", User::new("new-reviewer")));
    test.mock_pull_request_commits();
    let recipients = hint_recipients(&mut test, "pull_request_review", "dismissed").await;
    assert!(recipients.contains(&"past-reviewer".into()));
    assert!(recipients.contains(&"new-reviewer".into()));

    test.handler.data.review = None;
    test.mock_pull_request_commits();
    let recipients = hint_recipients(&mut test, "pull_request", "milestoned").await;
    assert!(recipients.contains(&"past-reviewer".into()));
    assert!(recipients.contains(&"new-reviewer".into()));
}

#[tokio::test]
async fn test_commits_cache() {
    let mut test = new_test();
    let commits = some_commits();
    let mut pr = some_pr().unwrap();
    pr.head.sha = commits.last().unwrap().sha.clone();
    test.handler.data.pull_request = Some(pr.clone());
    test.handler.data.sender = User::new("the-pr-owner");

    test.mock_pull_request_reviews(vec![]);
    test.mock_pull_request_commits();
    let recipients = hint_recipients(&mut test, "pull_request", "milestoned").await;
    assert!(recipients.contains(&"bob-author".into()));

    let recipients = hint_recipients(&mut test, "pull_request", "milestoned").await;
    assert!(recipients.contains(&"bob-author".into()));

    // A new head looks its commits up again.
    pr.head.sha = "abcabc".into();
    test.handler.data.pull_request = Some(pr);
    test.mock_pull_request_commits();
    hint_recipients(&mut test, "pull_request", "milestoned").await;
}

#[tokio::test]
async fn test_push_with_lagging_pr_skips_cached_commits() {
    let mut test = new_test();
    let commit = |sha: &str, author: &str| Commit {
        sha: sha.into(),
        html_url: format!("http://commit/{}", sha),
        author: Some(User::new(author)),
        commit: CommitDetails {
            message: "a commit".into(),
        },
    };
    let mut pr = some_pr().unwrap();
    pr.draft = Some(true);
    pr.head.sha = "the-before-commit".into();
    test.handler.data.pull_request = Some(pr.clone());
    test.handler.data.sender = User::new("the-pr-owner");

    // An earlier event caches the commits up to the push's before.
    let before = vec![commit("the-before-commit", "bob-author")];
    test.mock_pull_request_reviews(vec![]);
    test.github
        .mock_get_pull_request_commits("some-user", "some-repo", 32, Ok(before));
    hint_recipients(&mut test, "pull_request", "milestoned").await;

    // GitHub lists the PR before it has seen the push.
    test.handler.data.pull_request = None;
    test.handler.data.ref_name = Some("refs/heads/pr-branch".into());
    test.handler.data.before = Some("the-before-commit".into());
    test.handler.data.after = Some("the-after-commit".into());
    test.github.mock_get_pull_requests(
        "some-user",
        "some-repo",
        Some("open"),
        Some("pr-branch"),
        Ok(vec![pr]),
    );
    let after = vec![
        commit("the-before-commit", "bob-author"),
        commit("the-after-commit", "new-author"),
    ];
    test.github
        .mock_get_pull_request_commits("some-user", "some-repo", 32, Ok(after));
    let recipients = hint_recipients(&mut test, "push", "").await;
    assert!(recipients.contains(&"new-author".into()));
}

#[tokio::test]
async fn test_people_cache_drops_removed_people() {
    let mut test = new_test();
    test.handler.data.sender = User::new("the-pr-owner");
    let mut pr = some_pr().unwrap();
    pr.draft = Some(true);
    test.mock_pull_request_reviews(vec![]);

    pr.updated_at = Some("2026-11-13T10:00:00Z".into());
    test.handler.data.pull_request = Some(pr.clone());
    test.mock_pull_request_commits();
    hint_recipients(&mut test, "pull_request", "milestoned").await;

    pr.updated_at = Some("2026-11-13T10:00:01Z".into());
    pr.assignees = vec![];
    test.handler.data.pull_request = Some(pr.clone());
    test.mock_pull_request_commits();
    hint_recipients(&mut test, "pull_request", "unassigned").await;
    let people = test.handler.people_cache.get(&the_repo(), 32).unwrap();
    assert!(people.assignees.is_empty());
    assert_eq!(vec![User::new("joe-reviewer")], people.requested_reviewers);

    pr.updated_at = Some("2026-11-13T10:00:02Z".into());
    pr.requested_reviewers = Some(vec![]);
    test.handler.data.pull_request = Some(pr);
    test.handler.data.requested_reviewer = Some(User::new("joe-reviewer"));
    test.mock_pull_request_commits();
    hint_recipients(&mut test, "pull_request", "review_request_removed").await;
    let people = test.handler.people_cache.get(&the_repo(), 32).unwrap();
    assert!(people.requested_reviewers.is_empty());
}

#[tokio::test]
async fn test_people_cache() {
    let mut test = new_test();
    test.handler.data.sender = User::new("the-pr-owner");
    let pr = |updated_at: &str, assignee: &str| {
        let mut pr = some_pr().unwrap();
        pr.updated_at = Some(updated_at.into());
        pr.assignees = vec![User::new(assignee)];
        Some(pr)
    };
    let assignees = |test: &GithubHandlerTest| {
        let people = test.handler.people_cache.get(&the_repo(), 32).unwrap();
        people.assignees
    };
    test.mock_pull_request_reviews(vec![]);

    test.handler.data.pull_request = pr("2026-11-13T10:00:01Z", "newer");
    test.mock_pull_request_commits();
    hint_recipients(&mut test, "pull_request", "milestoned").await;
    assert_eq!(vec![User::new("newer")], assignees(&test));

    // A review webhook that GitHub sent earlier arrives late.
    test.handler.data.pull_request = pr("2026-11-13T10:00:00Z", "older");
    test.handler.data.review = Some(Review::new("", User::new("joe-reviewer")));
    test.mock_pull_request_commits();
    hint_recipients(&mut test, "pull_request_review", "dismissed").await;
    assert_eq!(vec![User::new("newer")], assignees(&test));

    test.handler.data.pull_request = pr("2026-11-13T10:00:02Z", "newest");
    test.mock_pull_request_commits();
    hint_recipients(&mut test, "pull_request_review", "dismissed").await;
    assert_eq!(vec![User::new("newest")], assignees(&test));
}

fn new_issue(key: &str) -> jira::Issue {
    jira::Issue {
        key: key.into(),
        fields: jira::Fields {
            status: jira::Status {
                name: "Triaged".to_string(),
            },
        },
    }
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
    vec![Commit {
        sha: "ffeedd00110011".into(),
        html_url: "http://commit/ffeedd00110011".into(),
        author: Some(User::new("bob-author")),
        commit: CommitDetails {
            message: "Fix [SER-1] Add the feature\n\nThe body ([OTHER-123])".into(),
        },
    }]
}

fn many_jira_commits() -> Vec<Commit> {
    let commit = Commit {
        sha: "ffeedd00110011".into(),
        html_url: "http://commit/ffeedd00110011".into(),
        author: Some(User::new("bob-author")),
        commit: CommitDetails {
            message: "Fix [SER-1] Add the feature\n\nThe body ([OTHER-123])".into(),
        },
    };

    (0..21)
        .collect::<Vec<u32>>()
        .into_iter()
        .map(|_| commit.clone())
        .collect()
}

fn some_jira_push_commits() -> Vec<PushCommit> {
    vec![PushCommit {
        id: "ffeedd00110011".into(),
        tree_id: "ffeedd00110011".into(),
        url: "http://commit/ffeedd00110011".into(),
        message: "Fix [SER-1] Add the feature\n\nThe body ([OTHER-123])".into(),
    }]
}

#[tokio::test]
async fn test_jira_pull_request_opened() {
    let mut test = new_test_with_jira();
    test.mock_pull_request_reviews(vec![]);
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

    test.slack.expect(vec![slack::req(
        SlackRecipient::by_name("the-reviews-channel"),
        &format!("{} {}", msg, REPO_MSG),
        &attach,
        Threading::Root("some-user/some-repo/32".to_string()),
    )]);

    expect_jira_ref_pass_pr(
        &test.github,
        some_pr().as_ref().unwrap(),
        &some_jira_commits(),
    );

    if let Some(ref jira) = test.jira {
        jira.mock_comment_issue(
            "SER-1",
            "Review submitted for branch master: http://the-pr",
            Ok(()),
        );

        jira.mock_get_issue("SER-1", Ok(new_issue("SER-1")));

        jira.mock_get_transitions("SER-1", Ok(vec![new_transition("001", "the-progress")]));
        jira.mock_transition_issue("SER-1", &new_transition_req("001"), Ok(()));

        jira.mock_get_transitions("SER-1", Ok(vec![new_transition("002", "the-review")]));
        jira.mock_transition_issue("SER-1", &new_transition_req("002"), Ok(()));
    }

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "pr".into()), resp);
}

#[tokio::test]
async fn test_jira_pull_request_opened_too_many_commits() {
    let mut test = new_test_with_jira();
    test.mock_pull_request_reviews(vec![]);
    test.handler.event = "pull_request".into();
    test.handler.action = "opened".into();
    test.handler.data.pull_request = some_pr();
    test.handler.data.sender = User::new("the-pr-owner");

    test.github.mock_get_pull_request_commits(
        "some-user",
        "some-repo",
        32,
        Ok(many_jira_commits()),
    );

    expect_jira_ref_pass_pr(
        &test.github,
        some_pr().as_ref().unwrap(),
        &some_jira_commits(),
    );

    let attach = vec![
        SlackAttachmentBuilder::new("")
            .title("Pull Request #32: \"The PR\"")
            .title_link("http://the-pr")
            .build(),
    ];

    test.slack.expect(vec![
        slack::req(
            SlackRecipient::by_name("the-reviews-channel"),
            &format!("Pull Request opened by the.pr.owner {}", REPO_MSG),
            &attach,
            Threading::Root("some-user/some-repo/32".to_string()),
        ),
        slack::req(
            SlackRecipient::by_name("the-reviews-channel"),
            &format!(
                "Too many commits on Pull Request #32. Ignoring JIRAs. {}",
                REPO_MSG
            ),
            &attach,
            Threading::Flat,
        ),
        slack::req(
            SlackRecipient::user_mention("the.pr.owner"),
            "Too many commits on Pull Request #32. Ignoring JIRAs.",
            &attach,
            Threading::Flat,
        ),
    ]);

    // do not set jira expectations

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "pr".into()), resp);
}

#[tokio::test]
async fn test_jira_push_master() {
    let mut test = new_test_with_jira();
    test.handler.event = "push".into();
    test.handler.data.ref_name = Some("refs/heads/master".into());
    test.handler.data.before = Some("abcdef0000".into());
    test.handler.data.after = Some("1111abcdef".into());
    let commits = some_jira_push_commits();
    test.handler.data.commits = Some(commits.clone());

    test.expect_will_run_version_script("master", "1111abcdef", &commits);

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "push".into()), resp);
}

#[tokio::test]
async fn test_jira_push_develop() {
    let mut test = new_test_with_jira();
    test.handler.event = "push".into();
    test.handler.data.ref_name = Some("refs/heads/develop".into());
    test.handler.data.before = Some("abcdef0000".into());
    test.handler.data.after = Some("1111abcdef".into());
    let commits = some_jira_push_commits();
    test.handler.data.commits = Some(commits.clone());

    test.expect_will_run_version_script("develop", "1111abcdef", &commits);

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "push".into()), resp);
}

#[tokio::test]
async fn test_jira_push_release() {
    let mut test = new_test_with_jira();
    test.handler.event = "push".into();
    test.handler.data.ref_name = Some("refs/heads/release/55".into());
    test.handler.data.before = Some("abcdef0000".into());
    test.handler.data.after = Some("1111abcdef".into());
    let commits = some_jira_push_commits();
    test.handler.data.commits = Some(commits.clone());

    test.expect_will_run_version_script("release/55", "1111abcdef", &commits);

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "push".into()), resp);
}

#[tokio::test]
async fn test_jira_push_other_branch() {
    let mut test = new_test_with_jira();
    test.handler.event = "push".into();
    test.handler.data.ref_name = Some("refs/heads/some-branch".into());
    test.handler.data.before = Some("abcdef0000".into());
    test.handler.data.after = Some("1111abcdef".into());

    test.handler.data.commits = Some(some_jira_push_commits());

    test.github.mock_get_pull_requests(
        "some-user",
        "some-repo",
        Some("open"),
        Some("some-branch"),
        Ok(vec![]),
    );

    // no jira mocks: will fail if called

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "push".into()), resp);
}

#[tokio::test]
async fn test_jira_disabled() {
    let mut test = new_test_with_jira();
    test.handler.event = "push".into();
    test.handler.data.ref_name = Some("refs/heads/master".into());
    test.handler.data.before = Some("abcdef0000".into());
    test.handler.data.after = Some("1111abcdef".into());
    test.handler.data.commits = Some(some_jira_push_commits());

    // change the repo to an unconfigured one
    test.handler.repository = Repo::parse(&format!(
        "http://{}/some-other-user/some-other-repo",
        test.github.github_host()
    ))
    .unwrap();

    // no jira mocks: will fail if called

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "push".into()), resp);
}

#[tokio::test]
async fn test_jira_push_triggers_version_script() {
    let mut test = new_test_with_jira();

    test.config
        .repos_write()
        .insert_info(
            &repos::RepoInfo::new("some-user/versioning-repo", "the-reviews-channel")
                .with_jira("PRJ"),
        )
        .expect("Failed to add repo");

    // change the repo to an unconfigured one
    test.handler.repository = Repo::parse(&format!(
        "http://{}/some-user/versioning-repo",
        test.github.github_host()
    ))
    .unwrap();

    test.handler.event = "push".into();
    test.handler.data.ref_name = Some("refs/heads/master".into());
    test.handler.data.before = Some("abcdef0000".into());
    test.handler.data.after = Some("1111abcdef".into());
    let commits = some_jira_push_commits();
    test.handler.data.commits = Some(commits.clone());

    test.expect_will_run_version_script("master", "1111abcdef", &commits);

    let resp = test.handler.handle_event().await.unwrap();
    assert_eq!((StatusCode::OK, "push".into()), resp);
}
