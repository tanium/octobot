use std::sync::{Arc, Mutex};

use log::{error, info};
use serde_derive::{Deserialize, Serialize};

use crate::util;
use crate::worker;
use octobot_lib::errors::*;
use octobot_lib::http_client::HTTPClient;
use octobot_lib::metrics::Metrics;

#[derive(Serialize, Clone, PartialEq, Eq, Debug)]
pub struct SlackAttachment {
    pub text: String,
    pub title: Option<String>,
    pub title_link: Option<String>,
    pub color: Option<String>,
}

#[derive(Deserialize)]
struct SlackResponse {
    ok: bool,
    error: Option<String>,
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
        SlackAttachmentBuilder {
            attachment: SlackAttachment::new(text),
        }
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

#[derive(Serialize, Clone, PartialEq)]
struct SlackMessage {
    text: String,
    attachments: Vec<SlackAttachment>,
    channel: String,
}

// the main object for sending messages to slack
struct Slack {
    client: Arc<HTTPClient>,
    recent_messages: Mutex<Vec<SlackMessage>>,
}

const TRIM_MESSAGES_AT: usize = 200;
const TRIM_MESSAGES_TO: usize = 20;

impl Slack {
    pub fn new(bot_token: String, metrics: Arc<Metrics>) -> Slack {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.append(
            reqwest::header::AUTHORIZATION,
            format!("Bearer {}", bot_token).parse().unwrap(),
        );

        let client = Arc::new(
            HTTPClient::new_with_headers("https://slack.com/api", headers)
                .unwrap()
                .with_metrics(
                    metrics.slack_api_responses.clone(),
                    metrics.slack_api_duration.clone(),
                ),
        );

        Slack {
            client,
            recent_messages: Mutex::new(Vec::new()),
        }
    }

    async fn send(&self, channel: &str, msg: &str, attachments: Vec<SlackAttachment>) {
        let slack_msg = SlackMessage {
            text: msg.to_string(),
            attachments: attachments,
            channel: channel.to_string(),
        };

        if !self.is_unique(&slack_msg) {
            info!("Skipping duplicate message to {}", channel);
            return;
        }

        info!("Sending message to #{}", channel);

        let res: Result<SlackResponse> = self.client.post("/chat.postMessage", &slack_msg).await;
        match res {
            Ok(r) => {
                if r.ok {
                    info!("Successfully sent slack message")
                } else {
                    error!(
                        "Error sending slack message: {}",
                        r.error.unwrap_or(String::new())
                    )
                }
            }
            Err(e) => error!("Error sending slack message: {}", e),
        }
    }

    fn is_unique(&self, req: &SlackMessage) -> bool {
        let mut recent_messages = self.recent_messages.lock().unwrap();
        util::check_unique_event(
            req.clone(),
            &mut *recent_messages,
            TRIM_MESSAGES_AT,
            TRIM_MESSAGES_TO,
        )
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

pub fn new_runner(
    bot_token: String,
    metrics: Arc<Metrics>,
) -> Arc<dyn worker::Runner<SlackRequest>> {
    Arc::new(Runner {
        slack: Arc::new(Slack::new(bot_token, metrics)),
    })
}

#[async_trait::async_trait]
impl worker::Runner<SlackRequest> for Runner {
    async fn handle(&self, req: SlackRequest) {
        self.slack
            .send(&req.channel, &req.msg, req.attachments)
            .await;
    }
}
