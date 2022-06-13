use std::sync::{Arc, Mutex};

use failure::bail;
use log::{debug, error, info};
use serde_derive::{Deserialize, Serialize};

use crate::slack_db::SlackDatabase;
use crate::util;
use crate::worker;
use octobot_lib::errors::*;
use octobot_lib::http_client::HTTPClient;
use octobot_lib::metrics::Metrics;
use octobot_lib::slack::SlackRecipient;

#[derive(Serialize, Clone, PartialEq, Eq, Debug)]
pub struct SlackAttachment {
    pub text: String,
    pub title: Option<String>,
    pub title_link: Option<String>,
    pub color: Option<String>,
    pub mrkdwn_in: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct SlackResponse {
    ok: bool,
    // ts is the message's ID that we can use to respond to messages
    // https://api.slack.com/methods/chat.postMessage#examples
    ts: Option<String>,
    error: Option<String>,
}

impl SlackAttachment {
    pub fn new(text: &str) -> SlackAttachment {
        SlackAttachment {
            text: text.to_string(),
            title: None,
            title_link: None,
            color: None,
            mrkdwn_in: None,
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

    pub fn markdown<S: Into<String>>(&mut self, value: S) -> &mut SlackAttachmentBuilder {
        self.attachment.text = value.into();
        self.attachment.mrkdwn_in = Some(vec!["text".into()]);
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
    // Provide a thread_ts value for the posted message to act as a reply to a parent message.
    // https://api.slack.com/methods/chat.postMessage#arg_thread_ts
    thread_ts: Option<String>,
}

// the main object for sending messages to slack
pub struct Slack {
    client: Arc<HTTPClient>,
    recent_messages: Mutex<Vec<SlackMessage>>,
    slack_db: SlackDatabase,
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
            slack_db: SlackDatabase::new("slack_db.sqlite3").unwrap(),
        }
    }

    pub async fn send(
        &self,
        channel_id: &str,
        channel_name: &str,
        msg: &str,
        attachments: Vec<SlackAttachment>,
        initial_thread: bool,
        thread_guid: &str,
    ) {
        let res = self
            .slack_db
            .lookup_previous_thread(thread_guid, channel_id)
            .await;
        let parent_thread = res.unwrap_or_default();

        let slack_msg = SlackMessage {
            text: msg.to_string(),
            attachments,
            channel: channel_id.to_string(),
            thread_ts: parent_thread.clone(),
        };

        if !self.is_unique(&slack_msg) {
            info!("Skipping duplicate message to {}", channel_name);
            return;
        }

        debug!("Sending message to #{}", channel_name);

        let res: Result<SlackResponse> = self.client.post("/chat.postMessage", &slack_msg).await;
        match res {
            Ok(r) => {
                if r.ok {
                    let thread = r.ts.unwrap_or_default();
                    info!(
                        "Successfully sent slack message to {}, ts: \"{}\"",
                        channel_name, thread
                    );
                    if initial_thread && !thread_guid.is_empty() && parent_thread.is_none() {
                        self.slack_db
                            .insert_thread(thread_guid, channel_id, thread.as_str())
                            .await
                            .ok();
                    }
                } else {
                    error!(
                        "Error sending slack message to {}: {} ({})",
                        channel_name,
                        channel_id,
                        r.error.unwrap_or_default(),
                    )
                }
            }
            Err(e) => error!(
                "Error sending slack message to {} ({}): {}",
                channel_name, channel_id, e
            ),
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

    pub async fn list_users(&self) -> Result<Vec<User>> {
        #[derive(Deserialize)]
        struct Resp {
            ok: bool,
            error: Option<String>,
            members: Option<Vec<User>>,
            response_metadata: Option<ResponseMetadata>,
        }

        let mut result: Vec<User> = vec![];
        let mut next_cursor = String::new();

        loop {
            let mut url = String::from("/users.list?limit=200");
            if !next_cursor.is_empty() {
                url += "&cursor=";
                url += &next_cursor;
            }

            let res: Resp = self.client.get(&url).await?;

            if !res.ok {
                bail!(
                    "Failed to list users: {}",
                    res.error.unwrap_or_else(|| String::from("<unknown error>"))
                );
            }

            if let Some(m) = res.members {
                result.extend(m);
            }

            next_cursor = String::new();
            if let Some(m) = res.response_metadata {
                if let Some(c) = m.next_cursor {
                    next_cursor = c;
                }
            }

            if next_cursor.is_empty() {
                break;
            }
        }

        Ok(result)
    }

    pub async fn lookup_user_by_email(&self, email: &str) -> Result<Option<User>> {
        #[derive(Deserialize)]
        struct Resp {
            ok: bool,
            error: Option<String>,
            user: Option<User>,
        }

        let resp: Resp = self
            .client
            .get(&format!("/users.lookupByEmail?email={}", email))
            .await?;

        if !resp.ok {
            if resp.error == Some(String::from("users_not_found")) {
                return Ok(None);
            }
            bail!(
                "Failed to lookup user: {}",
                resp.error
                    .unwrap_or_else(|| String::from("<unknown error>"))
            );
        }

        Ok(resp.user)
    }
}

#[derive(Debug, PartialEq, Clone)]
pub struct SlackRequest {
    pub channel: SlackRecipient,
    pub thread_guid: Option<String>,
    pub msg: String,
    pub attachments: Vec<SlackAttachment>,
    pub initial_thread: bool,
}

#[derive(Debug, PartialEq, Clone, Deserialize)]
pub struct User {
    pub id: String,
    pub name: String,
    pub profile: UserProfile,
}

#[derive(Debug, PartialEq, Clone, Deserialize)]
pub struct UserProfile {
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub display_name: String,
}

#[derive(Deserialize)]
struct ResponseMetadata {
    next_cursor: Option<String>,
}

struct Runner {
    slack: Arc<Slack>,
}

pub fn req(
    channel: SlackRecipient,
    msg: &str,
    attachments: &[SlackAttachment],
    thread_guid: Option<String>,
    initial_thread: bool,
) -> SlackRequest {
    SlackRequest {
        channel,
        thread_guid,
        msg: msg.into(),
        attachments: attachments.into(),
        initial_thread,
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
            .send(
                &req.channel.id,
                &req.channel.name,
                &req.msg,
                req.attachments,
                req.initial_thread,
                req.thread_guid.unwrap_or_default().as_str(),
            )
            .await;
    }
}
