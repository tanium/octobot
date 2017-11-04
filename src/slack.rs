use std::sync::{Arc, Mutex};

use futures::{Future, future};
use tokio_core::reactor::Remote;

use http_client::HTTPClient;
use util;
use worker;

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


#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct SlackMessage {
    text: String,
    attachments: Vec<SlackAttachment>,
    channel: String,
}

// the main object for sending messages to slack
struct Slack {
    client: HTTPClient,
    recent_messages: Mutex<Vec<SlackMessage>>,
}

const TRIM_MESSAGES_AT: usize = 200;
const TRIM_MESSAGES_TO: usize = 20;

impl Slack {
    pub fn new(core_remote: Remote, webhook_url: &str) -> Slack {
        let client = HTTPClient::new(core_remote, webhook_url).with_headers(hashmap!{
                "Content-Type" => "application/json".to_string(),
            });

        Slack {
            client: client,
            recent_messages: Mutex::new(Vec::new()),
        }
    }

    fn send(&self, msg: SlackMessage) {
        if !self.is_unique(&msg) {
            info!("Skipping duplicate message to {}", msg.channel);
            return;
        }

        info!("Sending message to #{}", msg.channel);

        self.client.spawn(self.client.post_void_async("", &msg).then(|res| {
            match res {
                Ok(Ok(_)) => info!("Successfully sent slack message"),
                Ok(Err(e)) => error!("Error sending slack message: {}", e),
                Err(_) => error!("Unknown error sending slack message"),
            };
            future::ok::<(), ()>(())
        }));
    }

    fn is_unique(&self, req: &SlackMessage) -> bool {
        let mut recent_messages = self.recent_messages.lock().unwrap();
        util::check_unique_event(req.clone(), &mut *recent_messages, TRIM_MESSAGES_AT, TRIM_MESSAGES_TO)
    }
}

#[derive(Debug, PartialEq, Clone)]
pub enum SlackRequest {
    Message(SlackMessage),
}

struct Runner {
    slack: Arc<Slack>,
}

pub fn req(channel: &str, msg: &str, attachments: Vec<SlackAttachment>) -> SlackRequest {
    SlackRequest::Message(SlackMessage {
        channel: channel.into(),
        text: msg.into(),
        attachments: attachments,
    })
}

pub fn new_worker(core_remote: Remote, webhook_url: &str) -> worker::Worker<SlackRequest> {
    worker::Worker::new(Runner { slack: Arc::new(Slack::new(core_remote, webhook_url)) })
}

impl worker::Runner<SlackRequest> for Runner {
    fn handle(&self, req: SlackRequest) {
        match req {
            SlackRequest::Message(msg) => self.slack.send(msg),
        }
    }
}
