use std::sync::Arc;

use super::github;
use super::slack::{self, SlackAttachment};
use super::repos::RepoConfig;
use super::users::UserConfig;


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

    fn send_to_channel(&self,
                       msg: &str,
                       attachments: &Vec<SlackAttachment>,
                       repo: &github::Repo);
}


#[derive(Clone)]
pub struct SlackMessenger {
    pub slack_webhook_url: String,
    pub users: Arc<UserConfig>,
    pub repos: Arc<RepoConfig>,
}

impl Messenger for SlackMessenger {
    fn send_to_all(&self,
                   msg: &str,
                   attachments: &Vec<SlackAttachment>,
                   item_owner: &github::User,
                   sender: &github::User,
                   repo: &github::Repo,
                   assignees: &Vec<github::User>) {
        self.send_to_channel(msg, attachments, repo);

        let mut users: Vec<github::User> = assignees.iter().map(|a| a.clone()).collect();

        if !users.iter().any(|u| u.login == item_owner.login) {
            users.push(item_owner.clone());
        }

        // make sure we do not send private message to author of that message
        users.retain(|u| u.login != sender.login);

        self.send_to_slackbots(users, repo, msg, attachments);
    }

    fn send_to_owner(&self,
                     msg: &str,
                     attachments: &Vec<SlackAttachment>,
                     item_owner: &github::User,
                     repo: &github::Repo) {
        self.send_to_channel(msg, attachments, repo);
        self.send_to_slackbots(vec![item_owner.clone()], repo, msg, attachments);
    }

    fn send_to_channel(&self,
                       msg: &str,
                       attachments: &Vec<SlackAttachment>,
                       repo: &github::Repo) {
        if let Some(channel) = self.repos.lookup_channel(repo) {
            self.send_to_slack(channel.as_str(), msg, attachments);
        }
    }
}

impl SlackMessenger {
    fn send_to_slack(&self, channel: &str, msg: &str, attachments: &Vec<SlackAttachment>) {
        if let Err(e) = slack::send(self.slack_webhook_url.as_str(),
                                    channel,
                                    msg,
                                    attachments.clone()) {
            error!("Error sending to slack: {:?}", e);
        }
    }

    fn send_to_slackbots(&self,
                         users: Vec<github::User>,
                         repo: &github::Repo,
                         msg: &str,
                         attachments: &Vec<SlackAttachment>) {
        for user in users {
            // TODO: desiresPeaceAndQuiet.
            let slack_ref = self.users.slack_user_ref(user.login.as_str(), repo);
            self.send_to_slack(slack_ref.as_str(), msg, attachments);
        }
    }
}
