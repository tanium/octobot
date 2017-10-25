use std::sync::{Arc, Mutex};

use config::Config;
use github;
use slack::{self, SlackAttachment, SlackRequest};
use users;
use util;
use worker::WorkSender;

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
    pub slack: WorkSender<SlackRequest>,
    recent_messages: Mutex<Vec<SlackRequest>>,
}

pub fn new(config: Arc<Config>, slack: WorkSender<SlackRequest>) -> Box<Messenger> {
    Box::new(SlackMessenger {
        slack: slack,
        config: config.clone(),
        recent_messages: Mutex::new(Vec::new()),
    })
}

const DND_MARKER: &'static str = "DO NOT DISTURB";

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

        self.send_to_slackbots(slackbots, repo, msg, attachments);
    }

    fn send_to_owner(
        &self,
        msg: &str,
        attachments: &Vec<SlackAttachment>,
        item_owner: &github::User,
        repo: &github::Repo,
    ) {
        self.send_to_channel(msg, attachments, repo);
        self.send_to_slackbots(vec![item_owner.clone()], repo, msg, attachments);
    }

    fn send_to_channel(&self, msg: &str, attachments: &Vec<SlackAttachment>, repo: &github::Repo) {
        if let Some(channel) = self.config.repos().lookup_channel(repo) {
            let channel_msg = format!("{} ({})", msg, util::make_link(&repo.html_url, &repo.full_name));
            self.send_to_slack(channel.as_str(), &channel_msg, attachments);
        }
    }
}

const TRIM_MESSAGES_AT: usize = 200;
const TRIM_MESSAGES_TO: usize = 20;

impl SlackMessenger {
    fn send_to_slack(&self, channel: &str, msg: &str, attachments: &Vec<SlackAttachment>) {
        // user desires peace and quiet. do not disturb!
        if channel == DND_MARKER || channel == users::mention(DND_MARKER) {
            return;
        }

        let req = slack::req(channel, msg, attachments.clone());
        if !self.is_unique(&req) {
            info!("Skipping duplicate message: {}", msg);
            return;
        }

        if let Err(e) = self.slack.send(req) {
            error!("Error sending to slack: {:?}", e);
        }
    }

    fn send_to_slackbots(
        &self,
        users: Vec<github::User>,
        repo: &github::Repo,
        msg: &str,
        attachments: &Vec<SlackAttachment>,
    ) {
        for user in users {
            let slack_ref = self.config.users().slack_user_ref(user.login(), repo);
            self.send_to_slack(slack_ref.as_str(), msg, attachments);
        }
    }

    fn is_unique(&self, req: &SlackRequest) -> bool {
        let mut recent_messages = self.recent_messages.lock().unwrap();
        util::check_unique_event(req.clone(), &mut *recent_messages, TRIM_MESSAGES_AT, TRIM_MESSAGES_TO)
    }
}
