extern crate octobot;
extern crate tempdir;

mod mocks;

use std::sync::Arc;

use tempdir::TempDir;

use octobot::config::Config;
use octobot::db::Database;
use octobot::github;
use octobot::messenger::{self, Messenger};
use octobot::users::UserInfo;
use octobot::slack;

use mocks::mock_slack::MockSlack;

fn new_messenger(slack: &MockSlack) -> Box<Messenger> {
    let emptydb = Database::new(":memory:").unwrap();
    messenger::new(Arc::new(Config::new(emptydb)), slack.new_sender())
}

#[test]
fn test_sends_to_owner() {
    let slack = MockSlack::new(vec![slack::req("@the.owner", "hello there", vec![])]);
    let messenger = new_messenger(&slack);
    messenger.send_to_all(
        "hello there",
        &vec![],
        &github::User::new("the-owner"),
        &github::User::new("the-sender"),
        &github::Repo::new(),
        &vec![],
    );
}

#[test]
fn test_sends_to_mapped_usernames() {
    let dir = TempDir::new("messenger_test.rs").unwrap();
    let db_file = dir.path().join("db.sqlite3");
    let db = Database::new(&db_file.to_string_lossy()).unwrap();

    let config = Arc::new(Config::new(db));
    config.users_write().insert("the-owner", "the-owners-slack-name").unwrap();

    let slack = MockSlack::new(vec![slack::req("@the-owners-slack-name", "hello there", vec![])]);
    let messenger = messenger::new(config, slack.new_sender());

    messenger.send_to_all(
        "hello there",
        &vec![],
        &github::User::new("the-owner"),
        &github::User::new("the-sender"),
        &github::Repo::parse("http://git.foo.com/some-org/some-repo").unwrap(),
        &vec![],
    );
}

#[test]
fn test_sends_to_owner_with_channel() {
    let dir = TempDir::new("messenger_test.rs").unwrap();
    let db_file = dir.path().join("db.sqlite3");
    let db = Database::new(&db_file.to_string_lossy()).unwrap();

    let config = Arc::new(Config::new(db));
    config.repos_write().insert("the-owner/the-repo", "the-review-channel").unwrap();

    // Note: it should put the repo name w/ link in the message
    let slack = MockSlack::new(vec![
        slack::req(
            "the-review-channel",
            "hello there (<http://git.foo.com/the-owner/the-repo|the-owner/the-repo>)",
            vec![]
        ),
        slack::req("@the.owner", "hello there", vec![]),
    ]);
    let messenger = messenger::new(config, slack.new_sender());

    messenger.send_to_all(
        "hello there",
        &vec![],
        &github::User::new("the-owner"),
        &github::User::new("the-sender"),
        &github::Repo::parse("http://git.foo.com/the-owner/the-repo").unwrap(),
        &vec![],
    );
}

#[test]
fn test_sends_to_assignees() {
    let slack = MockSlack::new(vec![
        slack::req("@the.owner", "hello there", vec![]),
        slack::req("@assign1", "hello there", vec![]),
        slack::req("@assign2", "hello there", vec![]),
    ]);
    let messenger = new_messenger(&slack);
    messenger.send_to_all(
        "hello there",
        &vec![],
        &github::User::new("the-owner"),
        &github::User::new("the-sender"),
        &github::Repo::new(),
        &vec![github::User::new("assign1"), github::User::new("assign2")],
    );
}

#[test]
fn test_does_not_send_to_event_sender() {
    let slack = MockSlack::new(vec![slack::req("@userB", "hello there", vec![])]);
    let messenger = new_messenger(&slack);
    // Note: 'userA' is owner, sender, and assignee. (i.e. commented on a PR that he opened and is
    // assigned to). Being sender excludes receipt from any of these messages.
    messenger.send_to_all(
        "hello there",
        &vec![],
        &github::User::new("userA"),
        &github::User::new("userA"),
        &github::Repo::new(),
        &vec![github::User::new("userA"), github::User::new("userB")],
    );
}

#[test]
fn test_sends_only_once() {
    let slack = MockSlack::new(vec![
        slack::req("@the.owner", "hello there", vec![]),
        slack::req("@assign2", "hello there", vec![]),
    ]);
    let messenger = new_messenger(&slack);
    // Note: 'the-owner' is also assigned. Should only receive one slackbot message though.
    messenger.send_to_all(
        "hello there",
        &vec![],
        &github::User::new("the-owner"),
        &github::User::new("the-sender"),
        &github::Repo::new(),
        &vec![github::User::new("the-owner"), github::User::new("assign2")],
    );
}

#[test]
fn test_peace_and_quiet() {
    let dir = TempDir::new("messenger_test.rs").unwrap();
    let db_file = dir.path().join("db.sqlite3");

    let config = Arc::new(Config::new(Database::new(&db_file.to_string_lossy()).unwrap()));

    let mut user = UserInfo::new("the-owner", "the.owner");
    user.mute_direct_messages = true;
    config.users_write().insert_info(&user).unwrap();

    // should not send to channel or to owner, only to asignee
    let slack = MockSlack::new(vec![slack::req("@assign2", "hello there", vec![])]);
    let messenger = messenger::new(config, slack.new_sender());

    messenger.send_to_all(
        "hello there",
        &vec![],
        &github::User::new("the-owner"),
        &github::User::new("the-sender"),
        &github::Repo::parse("http://git.foo.com/the-owner/the-repo").unwrap(),
        &vec![github::User::new("the-owner"), github::User::new("assign2")],
    );
}
