
use super::slack_hook::{Attachment, AttachmentBuilder, Slack, SlackText, PayloadBuilder};

use super::github;

pub trait Messenger: Send + Sync {
    fn send_to_all(&self,
                   msg: SlackText,
                   attachments: Vec<AttachmentBuilder>,
                   itemOwner: &github::User,
                   sender: &github::User,
                   repo: &github::Repo);

    fn send_to_owner(&self,
                     msg: SlackText,
                     attachments: Vec<AttachmentBuilder>,
                     itemOwner: &github::User,
                     repo: &github::Repo);

    fn send_to_channel(&self,
                       msg: SlackText,
                       attachments: Vec<AttachmentBuilder>,
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
                   msg: SlackText,
                   attachments: Vec<AttachmentBuilder>,
                   itemOwner: &github::User,
                   sender: &github::User,
                   repo: &github::Repo) {
        self.send_to_channel(msg, attachments, itemOwner, Some(sender), repo);
        // println!("SEND TO ALL: {}", msg);
    }

    fn send_to_owner(&self,
                     msg: SlackText,
                     attachments: Vec<AttachmentBuilder>,
                     itemOwner: &github::User,
                     repo: &github::Repo) {
        self.send_to_channel(msg, attachments, itemOwner, None, repo);
        // println!("SEND TO OWNER: {}", msg);
    }

    fn send_to_channel(&self,
                       msg: SlackText,
                       attachments: Vec<AttachmentBuilder>,
                       itemOwner: &github::User,
                       sender: Option<&github::User>,
                       repo: &github::Repo) {
        let slack = match Slack::new(self.slack_webhook_url.as_str()) {
            Ok(s) => s,
            Err(e) => {
                error!("Error constructing slack object: {}", e);
                return;
            }
        };
        if let Some(channel) = self.repos.lookup_channel(repo) {
            let attachments = attachments.into_iter()
                .map(|a| a.build())
                .filter(|a| a.is_ok())
                .map(|a| a.unwrap())
                .collect();

            let payload = PayloadBuilder::new()
                .text(msg)
                .attachments(attachments)
                .channel(channel)
                .build();
            let payload = match payload {
                Ok(p) => p,
                Err(e) => {
                    error!("Error constructing payload: {}", e);
                    return;
                }
            };
            match slack.send(&payload) {
                Ok(_) => (),
                Err(e) => {
                    error!("Error sending to slack: {:?}", e);
                }
            }
        }
    }
}
