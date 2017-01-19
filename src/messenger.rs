use std::sync::Arc;
use std::rc::Rc;

use super::github;
use super::slack::{SlackSender, SlackAttachment};
use super::repos::RepoConfig;
use super::users::{self, UserConfig};
use super::util;

pub trait Messenger {
    fn send_to_all(&self,
                   msg: &str,
                   attachments: &Vec<SlackAttachment>,
                   item_owner: &github::User,
                   sender: &github::User,
                   repo: &github::Repo,
                   assignees: &Vec<github::User>);

    fn send_to_owner(&self,
                     msg: &str,
                     attachments: &Vec<SlackAttachment>,
                     item_owner: &github::User,
                     repo: &github::Repo);

    fn send_to_channel(&self, msg: &str, attachments: &Vec<SlackAttachment>, repo: &github::Repo);
}


#[derive(Clone)]
pub struct SlackMessenger {
    pub users: Arc<UserConfig>,
    pub repos: Arc<RepoConfig>,
    pub slack: Rc<SlackSender>,
}

const DND_MARKER : &'static str= "DO NOT DISTURB";

impl Messenger for SlackMessenger {
    fn send_to_all(&self,
                   msg: &str,
                   attachments: &Vec<SlackAttachment>,
                   item_owner: &github::User,
                   sender: &github::User,
                   repo: &github::Repo,
                   assignees: &Vec<github::User>) {
        self.send_to_channel(msg, attachments, repo);

        let mut slackbots: Vec<github::User> = vec![item_owner.clone()];

        slackbots.extend(assignees.iter().filter(|a| a.login != item_owner.login).map(|a| a.clone()));

        // make sure we do not send private message to author of that message
        slackbots.retain(|u| u.login != sender.login && u.login() != "octobot");

        self.send_to_slackbots(slackbots, repo, msg, attachments);
    }

    fn send_to_owner(&self,
                     msg: &str,
                     attachments: &Vec<SlackAttachment>,
                     item_owner: &github::User,
                     repo: &github::Repo) {
        self.send_to_channel(msg, attachments, repo);
        self.send_to_slackbots(vec![item_owner.clone()], repo, msg, attachments);
    }

    fn send_to_channel(&self, msg: &str, attachments: &Vec<SlackAttachment>, repo: &github::Repo) {
        if let Some(channel) = self.repos.lookup_channel(repo) {
            let channel_msg = format!("{} ({})", msg, util::make_link(&repo.html_url, &repo.full_name));
            self.send_to_slack(channel.as_str(), &channel_msg, attachments);
        }
    }
}

impl SlackMessenger {
    fn send_to_slack(&self, channel: &str, msg: &str, attachments: &Vec<SlackAttachment>) {
        // user desires peace and quiet. do not disturb!
        if channel == DND_MARKER || channel == users::mention(DND_MARKER) {
            return;
        }

        if let Err(e) = self.slack.send(channel, msg, attachments.clone()) {
            error!("Error sending to slack: {:?}", e);
        }
    }

    fn send_to_slackbots(&self,
                         users: Vec<github::User>,
                         repo: &github::Repo,
                         msg: &str,
                         attachments: &Vec<SlackAttachment>) {
        for user in users {
            let slack_ref = self.users.slack_user_ref(user.login(), repo);
            self.send_to_slack(slack_ref.as_str(), msg, attachments);
        }
    }
}
