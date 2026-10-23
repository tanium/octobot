use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use anyhow::bail;
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
    slack_db: Mutex<SlackDatabase>,
}

const TRIM_MESSAGES_AT: usize = 200;
const TRIM_MESSAGES_TO: usize = 20;

impl Slack {
    pub fn new(bot_token: String, slack_db_path: String, metrics: Arc<Metrics>) -> Slack {
        let slack_db = SlackDatabase::new(&slack_db_path).expect("init slack db");

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
            slack_db: Mutex::new(slack_db),
            recent_messages: Mutex::new(Vec::new()),
        }
    }

    pub fn clean(&self, expiration: SystemTime) -> Result<()> {
        self.slack_db.lock().unwrap().clean(expiration)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn send(
        &self,
        channel_id: &str,
        channel_name: &str,
        msg: &str,
        attachments: Vec<SlackAttachment>,
        initial_thread: bool,
        thread_guid: &str,
        thread_root: Option<ThreadRoot>,
    ) {
        let mut parent_thread = self.lookup_thread(thread_guid, channel_id);

        if parent_thread.is_none()
            && !thread_guid.is_empty()
            && let Some(root) = thread_root
        {
            parent_thread = self
                .start_thread(channel_id, channel_name, root, thread_guid)
                .await;
        }

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

        let Some(thread) = self.post(&slack_msg, channel_name).await else {
            return;
        };

        if initial_thread && !thread_guid.is_empty() && parent_thread.is_none() {
            let slack_db = self.slack_db.lock().unwrap();
            slack_db
                .insert_thread(thread_guid, channel_id, &thread)
                .ok();
        }
    }

    fn lookup_thread(&self, thread_guid: &str, channel_id: &str) -> Option<String> {
        let slack_db = self.slack_db.lock().unwrap();
        match slack_db.lookup_previous_thread(thread_guid, channel_id) {
            Ok(r) => r,
            Err(e) => {
                error!("Error looking up slack thread: {}", e);
                None
            }
        }
    }

    async fn start_thread(
        &self,
        channel_id: &str,
        channel_name: &str,
        root: ThreadRoot,
        thread_guid: &str,
    ) -> Option<String> {
        let root_msg = SlackMessage {
            text: root.msg,
            attachments: root.attachments,
            channel: channel_id.to_string(),
            thread_ts: None,
        };
        let thread = self.post(&root_msg, channel_name).await?;

        let slack_db = self.slack_db.lock().unwrap();
        if slack_db
            .insert_thread(thread_guid, channel_id, &thread)
            .is_ok()
        {
            return Some(thread);
        }

        // A concurrent send started this thread first; reply to that one instead.
        match slack_db.lookup_previous_thread(thread_guid, channel_id) {
            Ok(Some(existing)) => Some(existing),
            _ => Some(thread),
        }
    }

    async fn post(&self, slack_msg: &SlackMessage, channel_name: &str) -> Option<String> {
        debug!("Sending message to #{}", channel_name);

        let res: Result<SlackResponse> = self.client.post("/chat.postMessage", slack_msg).await;
        match res {
            Ok(r) if r.ok => {
                let thread = r.ts.unwrap_or_default();
                info!(
                    "Successfully sent slack message to {}, ts: \"{}\"",
                    channel_name, thread
                );
                Some(thread)
            }
            Ok(r) => {
                error!(
                    "Error sending slack message to {}: {} ({})",
                    channel_name,
                    slack_msg.channel,
                    r.error.unwrap_or_default(),
                );
                None
            }
            Err(e) => {
                error!(
                    "Error sending slack message to {} ({}): {}",
                    channel_name, slack_msg.channel, e
                );
                None
            }
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
    pub thread_root: Option<ThreadRoot>,
}

impl SlackRequest {
    pub fn with_thread_root(self, thread_root: ThreadRoot) -> SlackRequest {
        SlackRequest {
            thread_root: Some(thread_root),
            ..self
        }
    }
}

#[derive(Debug, PartialEq, Clone)]
pub struct ThreadRoot {
    pub msg: String,
    pub attachments: Vec<SlackAttachment>,
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
        thread_root: None,
    }
}

pub fn new_runner(slack: Arc<Slack>) -> Arc<dyn worker::Runner<SlackRequest>> {
    Arc::new(Runner { slack })
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
                req.thread_root,
            )
            .await;
    }
}
