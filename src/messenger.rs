use std::sync::Arc;

use config::Config;
use github;
use slack::{self, SlackAttachment, SlackRequest};
use util;
use worker::Worker;

pub trait Messenger {
    fn send_to_all(
        &self,
        msg: &str,
        attachments: &Vec<SlackAttachment>,
        item_owner: &github::User,
        sender: &github::User,
        repo: &github::Repo,
        participants: &Vec<github::User>,
    );

    fn send_to_owner(
        &self,
        msg: &str,
        attachments: &Vec<SlackAttachment>,
        item_owner: &github::User,
        repo: &github::Repo,
    );

    fn send_to_channel(&self, msg: &str, attachments: &Vec<SlackAttachment>, repo: &github::Repo);
}


struct SlackMessenger {
    pub config: Arc<Config>,
    pub slack: Arc<Worker<SlackRequest>>,
}

pub fn new(config: Arc<Config>, slack: Arc<Worker<SlackRequest>>) -> impl Messenger {
    SlackMessenger {
        slack: slack.clone(),
        config: config.clone(),
    }
}

impl Messenger for SlackMessenger {
    fn send_to_all(
        &self,
        msg: &str,
        attachments: &Vec<SlackAttachment>,
        item_owner: &github::User,
        sender: &github::User,
        repo: &github::Repo,
        participants: &Vec<github::User>,
    ) {
        self.send_to_channel(msg, attachments, repo);

        let mut slackbots: Vec<github::User> = vec![item_owner.clone()];

        slackbots.extend(
            participants.iter().filter(|a| a.login != item_owner.login).map(|a| a.clone()),
        );

        // make sure we do not send private message to author of that message
        slackbots.retain(|u| u.login != sender.login && u.login() != "octobot");

        self.send_to_slackbots(slackbots, msg, attachments);
    }

    fn send_to_owner(
        &self,
        msg: &str,
        attachments: &Vec<SlackAttachment>,
        item_owner: &github::User,
        repo: &github::Repo,
    ) {
        self.send_to_channel(msg, attachments, repo);
        self.send_to_slackbots(vec![item_owner.clone()], msg, attachments);
    }

    fn send_to_channel(&self, msg: &str, attachments: &Vec<SlackAttachment>, repo: &github::Repo) {
        if let Some(channel) = self.config.repos().lookup_channel(repo) {
            let channel_msg = format!("{} ({})", msg, util::make_link(&repo.html_url, &repo.full_name));
            self.send_to_slack(channel.as_str(), &channel_msg, attachments);
        }
    }
}

impl SlackMessenger {
    fn send_to_slack(&self, channel: &str, msg: &str, attachments: &Vec<SlackAttachment>) {
        self.slack.send(slack::req(channel, msg, attachments.clone()));
    }

    fn send_to_slackbots(&self, users: Vec<github::User>, msg: &str, attachments: &Vec<SlackAttachment>) {
        for user in users {
            if let Some(slack_ref) = self.config.users().slack_user_mention(&user.login()) {
                self.send_to_slack(&slack_ref, msg, attachments);
            }
        }
    }
}
