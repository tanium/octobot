use std::sync::Arc;

use http_client::HTTPClient;
use worker;

// the main object for sending messages to slack
struct Slack {
    webhook_url: String,
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

impl Slack {
    pub fn new(webhook_url: &str) -> Slack {
        Slack {
            webhook_url: webhook_url.into(),
        }
    }

    fn send(&self, channel: &str, msg: &str, attachments: Vec<SlackAttachment>)
            -> Result<(), String> {
        let slack_msg = SlackMessage {
            text: msg.to_string(),
            attachments: attachments,
            channel: channel.to_string(),
        };

        info!("Sending message to #{}", channel);

        let client = HTTPClient::new(&self.webhook_url)
            .with_headers(hashmap!{
                "Content-Type" => "application/json".to_string(),
            });

        client.post_void("", &slack_msg)
    }
}

#[derive(Debug, PartialEq, Clone)]
pub struct SlackRequest {
    pub channel: String,
    pub msg: String,
    pub attachments: Vec<SlackAttachment>,
}

struct Runner {
    slack: Arc<Slack>,
}

pub fn req(channel: &str, msg: &str, attachments: Vec<SlackAttachment>) -> SlackRequest {
    SlackRequest {
        channel: channel.into(),
        msg: msg.into(),
        attachments: attachments,
    }
}

pub fn new_worker(webhook_url: &str) -> worker::Worker<SlackRequest> {
    worker::Worker::new(Runner {
        slack: Arc::new(Slack::new(webhook_url)),
    })
}

impl worker::Runner<SlackRequest> for Runner {
    fn handle(&self, req: SlackRequest) {
        if let Err(e) = self.slack.send(&req.channel, &req.msg, req.attachments.clone()) {
            error!("Error sending to slack: {}", e);
        }
    }
}
