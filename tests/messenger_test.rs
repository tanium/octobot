extern crate octobot;

mod mocks;

use std::sync::Arc;

use octobot::messenger::{self, Messenger};
use octobot::config::Config;
use octobot::repos::RepoConfig;
use octobot::users::UserConfig;
use octobot::github;
use octobot::slack;

use mocks::mock_slack::MockSlack;

fn new_messenger(slack: &MockSlack) -> Box<Messenger> {
    messenger::new(Arc::new(Config::new(UserConfig::new(), RepoConfig::new())), slack.new_sender())
}

#[test]
fn test_sends_to_owner() {
    let slack = MockSlack::new(vec![
        slack::req("@the.owner", "hello there", vec![]),
    ]);
    let messenger = new_messenger(&slack);
    messenger.send_to_all("hello there",
                          &vec![],
                          &github::User::new("the-owner"),
                          &github::User::new("the-sender"),
                          &github::Repo::new(),
                          &vec![]);
}

#[test]
fn test_sends_to_mapped_usernames() {
    let mut users = UserConfig::new();
    users.insert("git.foo.com", "the-owner", "the-owners-slack-name");
    let config = Arc::new(Config::new(users, RepoConfig::new()));

    let slack =
        MockSlack::new(vec![slack::req("@the-owners-slack-name", "hello there", vec![])]);
    let messenger = messenger::new(config, slack.new_sender());

    messenger.send_to_all("hello there",
                          &vec![],
                          &github::User::new("the-owner"),
                          &github::User::new("the-sender"),
                          &github::Repo::parse("http://git.foo.com/some-org/some-repo").unwrap(),
                          &vec![]);
}

#[test]
fn test_sends_to_owner_with_channel() {
    let mut repos = RepoConfig::new();
    repos.insert("git.foo.com", "the-owner/the-repo", "the-review-channel");
    let config = Arc::new(Config::new(UserConfig::new(), repos));

    // Note: it should put the repo name w/ link in the message
    let slack = MockSlack::new(vec![slack::req("the-review-channel",
                                                   "hello there \
                                                    (<http://git.foo.\
                                                    com/the-owner/the-repo|the-owner/the-repo>)",
                                                   vec![]),
                                    slack::req("@the.owner", "hello there", vec![])]);
    let messenger = messenger::new(config, slack.new_sender());

    messenger.send_to_all("hello there",
                          &vec![],
                          &github::User::new("the-owner"),
                          &github::User::new("the-sender"),
                          &github::Repo::parse("http://git.foo.com/the-owner/the-repo").unwrap(),
                          &vec![]);
}

#[test]
fn test_sends_to_assignees() {
    let slack = MockSlack::new(vec![slack::req("@the.owner", "hello there", vec![]),
                                    slack::req("@assign1", "hello there", vec![]),
                                    slack::req("@assign2", "hello there", vec![])]);
    let messenger = new_messenger(&slack);
    messenger.send_to_all("hello there",
                          &vec![],
                          &github::User::new("the-owner"),
                          &github::User::new("the-sender"),
                          &github::Repo::new(),
                          &vec![github::User::new("assign1"), github::User::new("assign2")]);
}

#[test]
fn test_does_not_send_to_event_sender() {
    let slack = MockSlack::new(vec![slack::req("@userB", "hello there", vec![])]);
    let messenger = new_messenger(&slack);
    // Note: 'userA' is owner, sender, and assignee. (i.e. commented on a PR that he opened and is
    // assigned to). Being sender excludes receipt from any of these messages.
    messenger.send_to_all("hello there",
                          &vec![],
                          &github::User::new("userA"),
                          &github::User::new("userA"),
                          &github::Repo::new(),
                          &vec![github::User::new("userA"), github::User::new("userB")]);
}

#[test]
fn test_sends_only_once() {
    let slack = MockSlack::new(vec![slack::req("@the.owner", "hello there", vec![]),
                                    slack::req("@assign2", "hello there", vec![])]);
    let messenger = new_messenger(&slack);
    // Note: 'the-owner' is also assigned. Should only receive one slackbot message though.
    messenger.send_to_all("hello there",
                          &vec![],
                          &github::User::new("the-owner"),
                          &github::User::new("the-sender"),
                          &github::Repo::new(),
                          &vec![github::User::new("the-owner"), github::User::new("assign2")]);
}

#[test]
fn test_peace_and_quiet() {
    let mut users = UserConfig::new();
    users.insert("git.foo.com", "the-owner", "DO NOT DISTURB");
    let mut repos = RepoConfig::new();
    repos.insert("git.foo.com", "the-owner/the-repo", "DO NOT DISTURB");
    let config = Arc::new(Config::new(users, repos));

    // should not send to channel or to owner, only to asignee
    let slack = MockSlack::new(vec![slack::req("@assign2", "hello there", vec![])]);
    let messenger = messenger::new(config, slack.new_sender());

    messenger.send_to_all("hello there",
                          &vec![],
                          &github::User::new("the-owner"),
                          &github::User::new("the-sender"),
                          &github::Repo::parse("http://git.foo.com/the-owner/the-repo").unwrap(),
                          &vec![github::User::new("the-owner"), github::User::new("assign2")]);
}

#[test]
fn test_does_not_send_duplicates() {
    let mut repos = RepoConfig::new();
    repos.insert("git.foo.com", "the-owner/the-repo", "the-review-channel");
    let config = Arc::new(Config::new(UserConfig::new(), repos));

    let slack = MockSlack::new(vec![
        slack::req("the-review-channel", "hello there (<http://git.foo.com/the-owner/the-repo|the-owner/the-repo>)", vec![]),
        slack::req("@the.owner", "hello there", vec![]),
        slack::req("@assign2", "hello there", vec![]),
    ]);

    let messenger = messenger::new(config, slack.new_sender());

    messenger.send_to_all("hello there",
                          &vec![],
                          &github::User::new("the-owner"),
                          &github::User::new("the-sender"),
                          &github::Repo::parse("http://git.foo.com/the-owner/the-repo").unwrap(),
                          &vec![github::User::new("the-owner"), github::User::new("assign2")]);

    // send it again w/ the same args -- should not send to owner, assignee, or channel again
    messenger.send_to_all("hello there",
                          &vec![],
                          &github::User::new("the-owner"),
                          &github::User::new("the-sender"),
                          &github::Repo::parse("http://git.foo.com/the-owner/the-repo").unwrap(),
                          &vec![github::User::new("the-owner"), github::User::new("assign2")]);
}
