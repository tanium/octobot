use std::io::Read;
use hyper;
use hyper::header::ContentType;
use hyper::mime::{Mime, TopLevel, SubLevel};
use serde_json;

// the main object for sending messages to slack
pub struct Slack {
    pub webhook_url: String,
}

pub trait SlackSender {
    fn send(&self, channel: &str, msg: &str, attachments: Vec<SlackAttachment>)
            -> Result<(), String>;
}

#[derive(Serialize, Clone, PartialEq, Eq, Debug)]
pub struct SlackAttachment {
    pub text: String,
    pub title: Option<String>,
    pub title_link: Option<String>,
    pub color: Option<String>,
}

impl SlackAttachment {
    pub fn new(text: &str) -> SlackAttachment {
        SlackAttachment {
            text: text.to_string(),
            title: None,
            title_link: None,
            color: None,
        }
    }
}

pub struct SlackAttachmentBuilder {
    attachment: SlackAttachment,
}

impl SlackAttachmentBuilder {
    pub fn new(text: &str) -> SlackAttachmentBuilder {
        SlackAttachmentBuilder { attachment: SlackAttachment::new(text) }
    }

    pub fn text<S: Into<String>>(&mut self, value: S) -> &mut SlackAttachmentBuilder {
        self.attachment.text = value.into();
        self
    }

    pub fn title<S: Into<String>>(&mut self, value: S) -> &mut SlackAttachmentBuilder {
        self.attachment.title = Some(value.into());
        self
    }
    pub fn title_link<S: Into<String>>(&mut self, value: S) -> &mut SlackAttachmentBuilder {
        self.attachment.title_link = Some(value.into());
        self
    }

    pub fn color<S: Into<String>>(&mut self, value: S) -> &mut SlackAttachmentBuilder {
        self.attachment.color = Some(value.into());
        self
    }

    pub fn build(&self) -> SlackAttachment {
        self.attachment.clone()
    }
}


#[derive(Serialize)]
struct SlackMessage {
    text: String,
    attachments: Vec<SlackAttachment>,
    channel: String,
}

impl SlackSender for Slack {
    fn send(&self, channel: &str, msg: &str, attachments: Vec<SlackAttachment>)
            -> Result<(), String> {
        let slack_msg = SlackMessage {
            text: msg.to_string(),
            attachments: attachments,
            channel: channel.to_string(),
        };

        info!("Sending message to #{}", channel);

        let client = hyper::client::Client::new();
        let res = client.post(self.webhook_url.as_str())
            .header(ContentType(Mime(TopLevel::Application, SubLevel::Json, vec![])))
            .body(serde_json::to_string(&slack_msg).unwrap_or(String::new()).as_str())
            .send();

        match res {
            Ok(mut res) => {
                if res.status == hyper::Ok {
                    Ok(())
                } else {
                    let mut res_str = String::new();
                    res.read_to_string(&mut res_str).unwrap_or(0);
                    Err(format!("Error sending to slack: HTTP {} -- {}", res.status, res_str))
                }
            }
            Err(e) => Err(format!("Error sending to slack: {}", e)),
        }
    }
}
