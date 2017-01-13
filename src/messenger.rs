use std::sync::Arc;

use super::github;
use super::slack::{self, SlackAttachment};
use super::repos::RepoConfig;
use super::users::UserConfig;


pub trait Messenger: Send + Sync {
    fn send_to_all(&self,
                   msg: &str,
                   attachments: &Vec<SlackAttachment>,
                   itemOwner: &github::User,
                   sender: &github::User,
                   repo: &github::Repo);

    fn send_to_owner(&self,
                     msg: &str,
                     attachments: &Vec<SlackAttachment>,
                     itemOwner: &github::User,
                     repo: &github::Repo);

    fn send_to_channel(&self,
                       msg: &str,
                       attachments: &Vec<SlackAttachment>,
                       itemOwner: &github::User,
                       sender: Option<&github::User>,
                       repo: &github::Repo);
}


#[derive(Clone)]
pub struct SlackMessenger {
    pub slack_webhook_url: String,
    pub users: Arc<UserConfig>,
    pub repos: Arc<RepoConfig>,
}

unsafe impl Send for SlackMessenger {}
unsafe impl Sync for SlackMessenger {}

impl Messenger for SlackMessenger {
    fn send_to_all(&self,
                   msg: &str,
                   attachments: &Vec<SlackAttachment>,
                   itemOwner: &github::User,
                   sender: &github::User,
                   repo: &github::Repo) {
        self.send_to_channel(msg, attachments, itemOwner, Some(sender), repo);
        println!("SEND TO ALL: {}", msg);
    }

    fn send_to_owner(&self,
                     msg: &str,
                     attachments: &Vec<SlackAttachment>,
                     itemOwner: &github::User,
                     repo: &github::Repo) {
        self.send_to_channel(msg, attachments, itemOwner, None, repo);
        // println!("SEND TO OWNER: {}", msg);
    }

    fn send_to_channel(&self,
                       msg: &str,
                       attachments: &Vec<SlackAttachment>,
                       itemOwner: &github::User,
                       sender: Option<&github::User>,
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
}
