mod mocks;

use std::sync::Arc;

use octobot_ops::messenger::{Participants, Thread};
use tempfile::{TempDir, tempdir};

use mocks::mock_slack::MockSlack;
use octobot_lib::config::Config;
use octobot_lib::config_db::ConfigDatabase;
use octobot_lib::github;
use octobot_lib::repos::RepoInfo;
use octobot_lib::slack::SlackRecipient;
use octobot_ops::messenger;
use octobot_ops::slack::{self, SlackAttachmentBuilder, ThreadRoot};

fn new_test() -> (Arc<Config>, TempDir) {
    let temp_dir = tempdir().unwrap();
    let db_file = temp_dir.path().join("db.sqlite3");
    let db = ConfigDatabase::new(&db_file.to_string_lossy()).expect("create temp database");

    let config = Arc::new(Config::new(db));
    config
        .users_write()
        .insert("the-owner", "the.owner")
        .unwrap();
    config
        .users_write()
        .insert("the-sender", "the.sender")
        .unwrap();

    (config, temp_dir)
}

#[test]
fn test_sends_to_owner() {
    let (config, _temp) = new_test();

    let slack = MockSlack::new(vec![slack::req(
        SlackRecipient::user_mention("the.owner"),
        "hello there",
        &[],
        None,
        false,
    )]);
    let messenger = messenger::new(config, slack.new_sender());
    messenger.send_to_all(
        "hello there",
        &[],
        &github::User::new("the-owner"),
        &github::User::new("the-sender"),
        &github::Repo::new(),
        Participants::new(),
        "",
        &Vec::<github::Commit>::new(),
        vec![Thread::new("some-user/some-repo/1")],
    );
}

#[test]
fn test_sends_to_mapped_usernames() {
    let (config, _temp) = new_test();

    let slack = MockSlack::new(vec![slack::req(
        SlackRecipient::user_mention("the.owner"),
        "hello there",
        &[],
        None,
        false,
    )]);
    let messenger = messenger::new(config, slack.new_sender());

    messenger.send_to_all(
        "hello there",
        &[],
        &github::User::new("the-owner"),
        &github::User::new("the-sender"),
        &github::Repo::parse("http://git.foo.com/some-org/some-repo").unwrap(),
        Participants::new(),
        "",
        &Vec::<github::Commit>::new(),
        vec![Thread::new("some-user/some-repo/1")],
    );
}

#[test]
fn test_sends_to_owner_with_channel() {
    let (config, _temp) = new_test();

    config
        .repos_write()
        .insert("the-owner/the-repo", "the-review-channel")
        .unwrap();

    // Note: it should put the repo name w/ link in the message
    let slack = MockSlack::new(vec![
        slack::req(
            SlackRecipient::by_name("the-review-channel"),
            "hello there (<http://git.foo.com/the-owner/the-repo|the-owner/the-repo>)",
            &[],
            None,
            false,
        ),
        slack::req(
            SlackRecipient::user_mention("the.owner"),
            "hello there",
            &[],
            None,
            false,
        ),
    ]);
    let messenger = messenger::new(config, slack.new_sender());

    messenger.send_to_all(
        "hello there",
        &[],
        &github::User::new("the-owner"),
        &github::User::new("the-sender"),
        &github::Repo::parse("http://git.foo.com/the-owner/the-repo").unwrap(),
        Participants::new(),
        "",
        &Vec::<github::Commit>::new(),
        vec![Thread::new("some-user/some-repo/1")],
    );
}

#[test]
fn test_sends_to_assignees() {
    let (config, _temp) = new_test();

    config.users_write().insert("assign1", "assign1").unwrap();
    config.users_write().insert("assign2", "assign2").unwrap();

    let slack = MockSlack::new(vec![
        slack::req(
            SlackRecipient::user_mention("assign1"),
            "hello there",
            &[],
            None,
            false,
        ),
        slack::req(
            SlackRecipient::user_mention("assign2"),
            "hello there",
            &[],
            None,
            false,
        ),
        slack::req(
            SlackRecipient::user_mention("the.owner"),
            "hello there",
            &[],
            None,
            false,
        ),
    ]);
    let messenger = messenger::new(config, slack.new_sender());
    let mut p = Participants::new();
    p.add_user(github::User::new("assign1"));
    p.add_user(github::User::new("assign2"));
    messenger.send_to_all(
        "hello there",
        &[],
        &github::User::new("the-owner"),
        &github::User::new("the-sender"),
        &github::Repo::new(),
        p,
        "",
        &Vec::<github::Commit>::new(),
        vec![Thread::new("some-user/some-repo/1")],
    );
}

#[test]
fn test_does_not_send_to_event_sender() {
    let (config, _temp) = new_test();

    config.users_write().insert("userA", "userA").unwrap();
    config.users_write().insert("userB", "userB").unwrap();

    let slack = MockSlack::new(vec![slack::req(
        SlackRecipient::user_mention("userB"),
        "hello there",
        &[],
        None,
        false,
    )]);
    let messenger = messenger::new(config, slack.new_sender());
    // Note: 'userA' is owner, sender, and assignee. (i.e. commented on a PR that he opened and is
    // assigned to). Being sender excludes receipt from any of these messages.
    let mut p = Participants::new();
    p.add_user(github::User::new("userA"));
    p.add_user(github::User::new("userB"));
    messenger.send_to_all(
        "hello there",
        &[],
        &github::User::new("userA"),
        &github::User::new("userA"),
        &github::Repo::new(),
        p,
        "",
        &Vec::<github::Commit>::new(),
        vec![Thread::new("some-user/some-repo/1")],
    );
}

#[test]
fn test_sends_only_once() {
    let (config, _temp) = new_test();

    config.users_write().insert("assign2", "assign2").unwrap();

    let slack = MockSlack::new(vec![
        slack::req(
            SlackRecipient::user_mention("assign2"),
            "hello there",
            &[],
            None,
            false,
        ),
        slack::req(
            SlackRecipient::user_mention("the.owner"),
            "hello there",
            &[],
            None,
            false,
        ),
    ]);
    let messenger = messenger::new(config, slack.new_sender());
    // Note: 'the-owner' is also assigned. Should only receive one slackbot message though.
    let mut p = Participants::new();
    p.add_user(github::User::new("the-owner"));
    p.add_user(github::User::new("assign2"));
    messenger.send_to_all(
        "hello there",
        &[],
        &github::User::new("the-owner"),
        &github::User::new("the-sender"),
        &github::Repo::new(),
        p,
        "",
        &Vec::<github::Commit>::new(),
        vec![Thread::new("some-user/some-repo/1")],
    );
}

#[test]
fn test_peace_and_quiet() {
    let (config, _temp) = new_test();

    config.users_write().insert("assign2", "assign2").unwrap();

    let mut user = config.users().lookup_info("the-owner").unwrap();
    user.mute_direct_messages = true;
    config.users_write().update(&user).unwrap();

    // should not send to channel or to owner, only to asignee
    let slack = MockSlack::new(vec![slack::req(
        SlackRecipient::user_mention("assign2"),
        "hello there",
        &[],
        None,
        false,
    )]);
    let messenger = messenger::new(config, slack.new_sender());

    let mut p = Participants::new();
    p.add_user(github::User::new("the-owner"));
    p.add_user(github::User::new("assign2"));
    messenger.send_to_all(
        "hello there",
        &[],
        &github::User::new("the-owner"),
        &github::User::new("the-sender"),
        &github::Repo::parse("http://git.foo.com/the-owner/the-repo").unwrap(),
        p,
        "",
        &Vec::<github::Commit>::new(),
        vec![Thread::new("some-user/some-repo/1")],
    );
}

#[test]
fn test_peace_and_quiet_teams() {
    let (config, _temp) = new_test();

    config.users_write().insert("assign2", "assign2").unwrap();

    let mut user = config.users().lookup_info("assign2").unwrap();
    user.mute_direct_messages = false;
    user.mute_team_direct_messages = true;
    config.users_write().update(&user).unwrap();

    let slack = MockSlack::new(vec![slack::req(
        SlackRecipient::user_mention("the.owner"),
        "hello there",
        &[],
        None,
        false,
    )]);
    let messenger = messenger::new(config, slack.new_sender());

    let mut p = Participants::new();
    p.add_team_member(github::User::new("assign1"));
    p.add_team_member(github::User::new("assign2"));
    messenger.send_to_all(
        "hello there",
        &[],
        &github::User::new("the-owner"),
        &github::User::new("the-sender"),
        &github::Repo::parse("http://git.foo.com/the-owner/the-repo").unwrap(),
        p,
        "",
        &Vec::<github::Commit>::new(),
        vec![Thread::new("some-user/some-repo/1")],
    );
}

fn thread_direct_messages(config: &Config, github_name: &str) {
    let mut user = config.users().lookup_info(github_name).unwrap();
    user.thread_direct_messages = true;
    config.users_write().update(&user).unwrap();
}

fn pr_thread(guid: &str) -> Thread {
    Thread::for_pull_request(guid, SlackAttachmentBuilder::new("").build())
}

#[test]
fn test_threads_direct_messages() {
    let (config, _temp) = new_test();
    thread_direct_messages(&config, "the-owner");

    let slack = MockSlack::new(vec![slack::req(
        SlackRecipient::user_mention("the.owner"),
        "hello there",
        &[],
        Some("some-user/some-repo/1".to_string()),
        true,
    )]);
    let messenger = messenger::new(config, slack.new_sender());
    messenger.send_to_all(
        "hello there",
        &[],
        &github::User::new("the-owner"),
        &github::User::new("the-sender"),
        &github::Repo::new(),
        Participants::new(),
        "",
        &Vec::<github::Commit>::new(),
        vec![
            pr_thread("some-user/some-repo/1"),
            pr_thread("some-user/some-repo/2"),
        ],
    );
}

#[test]
fn test_issue_direct_messages_not_threaded() {
    let (config, _temp) = new_test();
    thread_direct_messages(&config, "the-owner");

    let slack = MockSlack::new(vec![slack::req(
        SlackRecipient::user_mention("the.owner"),
        "hello there",
        &[],
        None,
        false,
    )]);
    let messenger = messenger::new(config, slack.new_sender());
    messenger.send_to_all(
        "hello there",
        &[],
        &github::User::new("the-owner"),
        &github::User::new("the-sender"),
        &github::Repo::new(),
        Participants::new(),
        "",
        &Vec::<github::Commit>::new(),
        vec![Thread::new("some-user/some-repo/1")],
    );
}

#[test]
fn test_threads_direct_messages_per_user() {
    let (config, _temp) = new_test();
    config.users_write().insert("assign1", "assign1").unwrap();
    thread_direct_messages(&config, "assign1");

    let slack = MockSlack::new(vec![
        slack::req(
            SlackRecipient::user_mention("assign1"),
            "hello there",
            &[],
            Some("some-user/some-repo/1".to_string()),
            true,
        ),
        slack::req(
            SlackRecipient::user_mention("the.owner"),
            "hello there",
            &[],
            None,
            false,
        ),
    ]);
    let messenger = messenger::new(config, slack.new_sender());

    let mut p = Participants::new();
    p.add_user(github::User::new("assign1"));
    messenger.send_to_all(
        "hello there",
        &[],
        &github::User::new("the-owner"),
        &github::User::new("the-sender"),
        &github::Repo::new(),
        p,
        "",
        &Vec::<github::Commit>::new(),
        vec![pr_thread("some-user/some-repo/1")],
    );
}

#[test]
fn test_send_to_owner_not_threaded() {
    let (config, _temp) = new_test();
    thread_direct_messages(&config, "the-owner");

    let slack = MockSlack::new(vec![slack::req(
        SlackRecipient::user_mention("the.owner"),
        "hello there",
        &[],
        None,
        false,
    )]);
    let messenger = messenger::new(config, slack.new_sender());
    messenger.send_to_owner(
        "hello there",
        &[],
        &github::User::new("the-owner"),
        &github::Repo::new(),
        "",
        &Vec::<github::Commit>::new(),
    );
}

fn threaded_channel_test() -> (Arc<Config>, TempDir, github::Repo, Thread, ThreadRoot) {
    let (config, temp) = new_test();
    config
        .repos_write()
        .insert_info(
            &RepoInfo::new("the-owner/the-repo", "the-review-channel").with_use_threads(true),
        )
        .unwrap();

    let pr = SlackAttachmentBuilder::new("")
        .title("Pull Request #1: \"The PR\"")
        .title_link("http://the-pr")
        .build();
    let root = ThreadRoot {
        msg: "New thread for Pull Request (<http://git.foo.com/the-owner/the-repo|the-owner/the-repo>)".into(),
        attachments: vec![pr.clone()],
    };
    let repo = github::Repo::parse("http://git.foo.com/the-owner/the-repo").unwrap();

    (
        config,
        temp,
        repo,
        Thread::for_pull_request("the-owner/the-repo/1", pr),
        root,
    )
}

#[test]
fn test_channel_thread_root() {
    let (config, _temp, repo, thread, root) = threaded_channel_test();

    let slack = MockSlack::new(vec![
        slack::req(
            SlackRecipient::by_name("the-review-channel"),
            "hello there (<http://git.foo.com/the-owner/the-repo|the-owner/the-repo>)",
            &[],
            Some("the-owner/the-repo/1".to_string()),
            false,
        )
        .with_thread_root(root),
        slack::req(
            SlackRecipient::user_mention("the.owner"),
            "hello there",
            &[],
            None,
            false,
        ),
    ]);
    let messenger = messenger::new(config, slack.new_sender());
    messenger.send_to_all(
        "hello there",
        &[],
        &github::User::new("the-owner"),
        &github::User::new("the-sender"),
        &repo,
        Participants::new(),
        "",
        &Vec::<github::Commit>::new(),
        vec![thread],
    );
}

#[test]
fn test_channel_initial_thread_has_no_root() {
    let (config, _temp, repo, thread, _root) = threaded_channel_test();

    let slack = MockSlack::new(vec![slack::req(
        SlackRecipient::by_name("the-review-channel"),
        "opened (<http://git.foo.com/the-owner/the-repo|the-owner/the-repo>)",
        &[],
        Some("the-owner/the-repo/1".to_string()),
        true,
    )]);
    let messenger = messenger::new(config, slack.new_sender());
    messenger.send_to_channel(
        "opened",
        &[],
        &repo,
        "",
        &Vec::<github::Commit>::new(),
        vec![thread],
        true,
    );
}

#[test]
fn test_thread_without_pull_request_has_no_root() {
    let (config, _temp, repo, _thread, _root) = threaded_channel_test();

    let slack = MockSlack::new(vec![slack::req(
        SlackRecipient::by_name("the-review-channel"),
        "hello there (<http://git.foo.com/the-owner/the-repo|the-owner/the-repo>)",
        &[],
        Some("the-owner/the-repo/1".to_string()),
        false,
    )]);
    let messenger = messenger::new(config, slack.new_sender());
    messenger.send_to_channel(
        "hello there",
        &[],
        &repo,
        "",
        &Vec::<github::Commit>::new(),
        vec![Thread::new("the-owner/the-repo/1")],
        false,
    );
}
