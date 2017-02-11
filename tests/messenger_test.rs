extern crate octobot;

mod mocks;

use std::rc::Rc;
use std::sync::Arc;

use octobot::messenger::{Messenger, SlackMessenger};
use octobot::config::Config;
use octobot::repos::RepoConfig;
use octobot::users::UserConfig;
use octobot::github;

use mocks::mock_slack::{SlackCall, MockSlack};

fn new_messenger(slack: MockSlack) -> SlackMessenger {
    SlackMessenger {
        config: Arc::new(Config::new(UserConfig::new(), RepoConfig::new())),
        slack: Rc::new(slack),
    }
}


#[test]
fn test_sends_to_owner() {
    let slack = MockSlack::new(vec![SlackCall::new("@the.owner", "hello there", vec![])]);
    let messenger = new_messenger(slack);
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

    let slack =
        MockSlack::new(vec![SlackCall::new("@the-owners-slack-name", "hello there", vec![])]);

    let messenger = SlackMessenger {
        config: Arc::new(Config::new(users, RepoConfig::new())),
        slack: Rc::new(slack),
    };

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

    // Note: it should put the repo name w/ link in the message
    let slack = MockSlack::new(vec![SlackCall::new("the-review-channel",
                                                   "hello there \
                                                    (<http://git.foo.\
                                                    com/the-owner/the-repo|the-owner/the-repo>)",
                                                   vec![]),
                                    SlackCall::new("@the.owner", "hello there", vec![])]);

    let messenger = SlackMessenger {
        config: Arc::new(Config::new(UserConfig::new(), repos)),
        slack: Rc::new(slack),
    };

    messenger.send_to_all("hello there",
                          &vec![],
                          &github::User::new("the-owner"),
                          &github::User::new("the-sender"),
                          &github::Repo::parse("http://git.foo.com/the-owner/the-repo").unwrap(),
                          &vec![]);
}

#[test]
fn test_sends_to_assignees() {
    let slack = MockSlack::new(vec![SlackCall::new("@the.owner", "hello there", vec![]),
                                    SlackCall::new("@assign1", "hello there", vec![]),
                                    SlackCall::new("@assign2", "hello there", vec![])]);
    let messenger = new_messenger(slack);
    messenger.send_to_all("hello there",
                          &vec![],
                          &github::User::new("the-owner"),
                          &github::User::new("the-sender"),
                          &github::Repo::new(),
                          &vec![github::User::new("assign1"), github::User::new("assign2")]);
}

#[test]
fn test_does_not_send_to_event_sender() {
    let slack = MockSlack::new(vec![SlackCall::new("@userB", "hello there", vec![])]);
    let messenger = new_messenger(slack);
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
    let slack = MockSlack::new(vec![SlackCall::new("@the.owner", "hello there", vec![]),
                                    SlackCall::new("@assign2", "hello there", vec![])]);
    let messenger = new_messenger(slack);
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

    // should not send to channel or to owner, only to asignee
    let slack = MockSlack::new(vec![SlackCall::new("@assign2", "hello there", vec![])]);

    let messenger = SlackMessenger {
        config: Arc::new(Config::new(users, repos)),
        slack: Rc::new(slack),
    };

    messenger.send_to_all("hello there",
                          &vec![],
                          &github::User::new("the-owner"),
                          &github::User::new("the-sender"),
                          &github::Repo::parse("http://git.foo.com/the-owner/the-repo").unwrap(),
                          &vec![github::User::new("the-owner"), github::User::new("assign2")]);
}
