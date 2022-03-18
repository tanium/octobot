mod mocks;

use std::sync::Arc;

use tempdir::TempDir;

use mocks::mock_slack::MockSlack;
use octobot_lib::config::Config;
use octobot_lib::config_db::ConfigDatabase;
use octobot_lib::github;
use octobot_ops::messenger;
use octobot_ops::slack;

fn new_test() -> (Arc<Config>, TempDir) {
    let temp_dir = TempDir::new("repos.rs").unwrap();
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

    let slack = MockSlack::new(vec![slack::req_id(
        "@the.owner",
        "the.owner",
        "hello there",
        &[],
    )]);
    let messenger = messenger::new(config, slack.new_sender());
    messenger.send_to_all(
        "hello there",
        &[],
        &github::User::new("the-owner"),
        &github::User::new("the-sender"),
        &github::Repo::new(),
        &[],
        "",
        &Vec::<github::Commit>::new(),
    );
}

#[test]
fn test_sends_to_mapped_usernames() {
    let (config, _temp) = new_test();

    let slack = MockSlack::new(vec![slack::req_id(
        "@the.owner",
        "the.owner",
        "hello there",
        &[],
    )]);
    let messenger = messenger::new(config, slack.new_sender());

    messenger.send_to_all(
        "hello there",
        &[],
        &github::User::new("the-owner"),
        &github::User::new("the-sender"),
        &github::Repo::parse("http://git.foo.com/some-org/some-repo").unwrap(),
        &[],
        "",
        &Vec::<github::Commit>::new(),
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
            "the-review-channel",
            "hello there (<http://git.foo.com/the-owner/the-repo|the-owner/the-repo>)",
            &[],
        ),
        slack::req_id("@the.owner", "the.owner", "hello there", &[]),
    ]);
    let messenger = messenger::new(config, slack.new_sender());

    messenger.send_to_all(
        "hello there",
        &[],
        &github::User::new("the-owner"),
        &github::User::new("the-sender"),
        &github::Repo::parse("http://git.foo.com/the-owner/the-repo").unwrap(),
        &[],
        "",
        &Vec::<github::Commit>::new(),
    );
}

#[test]
fn test_sends_to_assignees() {
    let (config, _temp) = new_test();

    config.users_write().insert("assign1", "assign1").unwrap();
    config.users_write().insert("assign2", "assign2").unwrap();

    let slack = MockSlack::new(vec![
        slack::req_id("@the.owner", "the.owner", "hello there", &[]),
        slack::req_id("@assign1", "assign1", "hello there", &[]),
        slack::req_id("@assign2", "assign2", "hello there", &[]),
    ]);
    let messenger = messenger::new(config, slack.new_sender());
    messenger.send_to_all(
        "hello there",
        &[],
        &github::User::new("the-owner"),
        &github::User::new("the-sender"),
        &github::Repo::new(),
        &[github::User::new("assign1"), github::User::new("assign2")],
        "",
        &Vec::<github::Commit>::new(),
    );
}

#[test]
fn test_does_not_send_to_event_sender() {
    let (config, _temp) = new_test();

    config.users_write().insert("userA", "userA").unwrap();
    config.users_write().insert("userB", "userB").unwrap();

    let slack = MockSlack::new(vec![slack::req_id("@userB", "userB", "hello there", &[])]);
    let messenger = messenger::new(config, slack.new_sender());
    // Note: 'userA' is owner, sender, and assignee. (i.e. commented on a PR that he opened and is
    // assigned to). Being sender excludes receipt from any of these messages.
    messenger.send_to_all(
        "hello there",
        &[],
        &github::User::new("userA"),
        &github::User::new("userA"),
        &github::Repo::new(),
        &[github::User::new("userA"), github::User::new("userB")],
        "",
        &Vec::<github::Commit>::new(),
    );
}

#[test]
fn test_sends_only_once() {
    let (config, _temp) = new_test();

    config.users_write().insert("assign2", "assign2").unwrap();

    let slack = MockSlack::new(vec![
        slack::req_id("@the.owner", "the.owner", "hello there", &[]),
        slack::req_id("@assign2", "assign2", "hello there", &[]),
    ]);
    let messenger = messenger::new(config, slack.new_sender());
    // Note: 'the-owner' is also assigned. Should only receive one slackbot message though.
    messenger.send_to_all(
        "hello there",
        &[],
        &github::User::new("the-owner"),
        &github::User::new("the-sender"),
        &github::Repo::new(),
        &[github::User::new("the-owner"), github::User::new("assign2")],
        "",
        &Vec::<github::Commit>::new(),
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
    let slack = MockSlack::new(vec![slack::req_id(
        "@assign2",
        "assign2",
        "hello there",
        &[],
    )]);
    let messenger = messenger::new(config, slack.new_sender());

    messenger.send_to_all(
        "hello there",
        &[],
        &github::User::new("the-owner"),
        &github::User::new("the-sender"),
        &github::Repo::parse("http://git.foo.com/the-owner/the-repo").unwrap(),
        &[github::User::new("the-owner"), github::User::new("assign2")],
        "",
        &Vec::<github::Commit>::new(),
    );
}
