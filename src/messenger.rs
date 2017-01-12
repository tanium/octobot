
use super::slack_hook::{AttachmentBuilder, Slack, PayloadBuilder};

use super::github;

pub trait Messenger: Send + Sync {
    fn send_to_all(&self,
                   msg: &str,
                   attachments: &Vec<AttachmentBuilder>,
                   itemOwner: &github::User,
                   sender: &github::User,
                   repo: &github::Repo);

    fn send_to_owner(&self,
                     msg: &str,
                     attachments: &Vec<AttachmentBuilder>,
                     itemOwner: &github::User,
                     repo: &github::Repo);

    fn send_to_channel(&self,
                       msg: &str,
                       attachments: &Vec<AttachmentBuilder>,
                       itemOwner: &github::User,
                       sender: Option<&github::User>,
                       repo: &github::Repo);
}


#[derive(Clone)]
pub struct SlackMessenger {
    pub slack_webhook_url: String,
}

unsafe impl Send for SlackMessenger {}
unsafe impl Sync for SlackMessenger {}


impl Messenger for SlackMessenger {
    fn send_to_all(&self,
                       msg: &str,
                       attachments: &Vec<AttachmentBuilder>,
                       itemOwner: &github::User,
                       sender: &github::User,
                       repo: &github::Repo) {
        self.send_to_channel(msg, attachments.clone(), itemOwner, Some(sender), repo);
        println!("SEND TO ALL: {}", msg);
    }

    fn send_to_owner(&self,
                         msg: &str,
                         attachments: &Vec<AttachmentBuilder>,
                         itemOwner: &github::User,
                         repo: &github::Repo) {
        self.send_to_channel(msg, attachments.clone(), itemOwner, None, repo);
        println!("SEND TO OWNER: {}", msg);
    }

    fn send_to_channel(&self,
                           msg: &str,
                           attachments: &Vec<AttachmentBuilder>,
                           itemOwner: &github::User,
                           sender: Option<&github::User>,
                           repo: &github::Repo) {
        println!("SEND TO CHANNEL: {}", msg);
    }
}
