use super::std::io::Read;
use super::hyper;
use super::hyper::header::ContentType;
use super::hyper::mime::{Mime, TopLevel, SubLevel};
use super::rustc_serialize::json;

// the main object for sending messages to slack
pub struct Slack {
    pub webhook_url: String,
}

pub trait SlackSender {
    fn send(&self,
            channel: &str,
            msg: &str,
            attachments: Vec<SlackAttachment>)
            -> Result<(), String>;
}

#[derive(RustcEncodable, Clone)]
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


#[derive(RustcEncodable)]
struct SlackMessage {
    text: String,
    attachments: Vec<SlackAttachment>,
    channel: String,
}

impl SlackSender for Slack {
    fn send(&self,
            channel: &str,
            msg: &str,
            attachments: Vec<SlackAttachment>)
            -> Result<(), String> {
        let slack_msg = SlackMessage {
            text: msg.to_string(),
            attachments: attachments,
            channel: channel.to_string(),
        };

        let client = hyper::client::Client::new();
        let res = client.post(self.webhook_url.as_str())
            .header(ContentType(Mime(TopLevel::Application, SubLevel::Json, vec![])))
            .body(json::encode(&slack_msg).unwrap_or(String::new()).as_str())
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
