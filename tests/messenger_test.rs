extern crate octobot;

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use octobot::messenger::{Messenger, SlackMessenger};
use octobot::slack::{SlackSender, SlackAttachment};
use octobot::repos::RepoConfig;
use octobot::users::UserConfig;
use octobot::github;

struct SlackCall {
    pub channel: String,
    pub msg: String,
    pub attachments: Vec<SlackAttachment>,
}

impl SlackCall {
    pub fn new(channel: &str, msg: &str, attach: Vec<SlackAttachment>) -> SlackCall {
        SlackCall {
            channel: channel.to_string(),
            msg: msg.to_string(),
            attachments: attach,
        }
    }
}

struct MockSlack {
    pub expected_calls: Vec<SlackCall>,
    pub call_count: Cell<usize>,
    pub should_fail: Option<String>,
}

impl MockSlack {
    pub fn new(calls: Vec<SlackCall>) -> MockSlack {
        MockSlack {
            should_fail: None,
            call_count: Cell::new(0),
            expected_calls: calls,
        }
    }
}

impl SlackSender for MockSlack {
    fn send(&self,
            channel: &str,
            msg: &str,
            attachments: Vec<SlackAttachment>)
            -> Result<(), String> {

        if self.call_count.get() >= self.expected_calls.len() {
            panic!("Failed: received unexpected slack call");
        }

        assert_eq!(self.expected_calls[self.call_count.get()].channel, channel);
        assert_eq!(self.expected_calls[self.call_count.get()].msg, msg);
        assert_eq!(self.expected_calls[self.call_count.get()].attachments,
                   attachments);
        self.call_count.set(self.call_count.get() + 1);

        if let Some(ref msg) = self.should_fail {
            Err(msg.clone())
        } else {
            Ok(())
        }
    }
}

impl Drop for MockSlack {
    fn drop(&mut self) {
        if self.call_count.get() != self.expected_calls.len() {
            panic!("Failed: Expected {} calls but only received {}",
                   self.expected_calls.len(),
                   self.call_count.get());
        }
    }
}



fn new_messenger(slack: MockSlack) -> SlackMessenger {
    SlackMessenger {
        users: Arc::new(UserConfig::new()),
        repos: Arc::new(RepoConfig::new()),
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
        users: Arc::new(users),
        repos: Arc::new(RepoConfig::new()),
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
        users: Arc::new(UserConfig::new()),
        repos: Arc::new(repos),
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
