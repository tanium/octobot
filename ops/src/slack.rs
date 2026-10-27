use std::collections::HashMap;
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
    thread_locks: Mutex<HashMap<ThreadKey, Arc<tokio::sync::Mutex<()>>>>,
}

type ThreadKey = (String, String);

enum Posted {
    Sent(String),
    Rejected,
    Failed,
}

const TRIM_MESSAGES_AT: usize = 200;
const TRIM_MESSAGES_TO: usize = 20;

impl Slack {
    pub fn new(bot_token: String, slack_db_path: String, metrics: Arc<Metrics>) -> Slack {
        Slack::new_with_url("https://slack.com/api", bot_token, slack_db_path, metrics)
    }

    pub fn new_with_url(
        api_base: &str,
        bot_token: String,
        slack_db_path: String,
        metrics: Arc<Metrics>,
    ) -> Slack {
        let slack_db = SlackDatabase::new(&slack_db_path).expect("init slack db");

        let mut headers = reqwest::header::HeaderMap::new();
        headers.append(
            reqwest::header::AUTHORIZATION,
            format!("Bearer {}", bot_token).parse().unwrap(),
        );

        let client = Arc::new(
            HTTPClient::new_with_headers(api_base, headers)
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
            thread_locks: Mutex::new(HashMap::new()),
        }
    }

    pub fn clean(&self, expiration: SystemTime) -> Result<()> {
        self.slack_db.lock().unwrap().clean(expiration)
    }

    pub async fn send(
        &self,
        channel_id: &str,
        channel_name: &str,
        msg: &str,
        attachments: Vec<SlackAttachment>,
        threading: Threading,
    ) {
        let Some(thread_guid) = threading.guid().map(str::to_string) else {
            let slack_msg = SlackMessage {
                text: msg.to_string(),
                attachments,
                channel: channel_id.to_string(),
                thread_ts: None,
            };
            self.post_unique(&slack_msg, channel_name).await;
            return;
        };

        // Held from lookup through save so concurrent sends for one PR agree on its thread.
        let key = (thread_guid.clone(), channel_id.to_string());
        let lock = self
            .thread_locks
            .lock()
            .unwrap()
            .entry(key.clone())
            .or_default()
            .clone();
        {
            let _guard = lock.lock().await;
            self.send_threaded(
                channel_id,
                channel_name,
                msg,
                attachments,
                &thread_guid,
                threading,
            )
            .await;
        }

        let mut locks = self.thread_locks.lock().unwrap();
        if Arc::strong_count(&lock) == 2 {
            locks.remove(&key);
        }
    }

    async fn send_threaded(
        &self,
        channel_id: &str,
        channel_name: &str,
        msg: &str,
        attachments: Vec<SlackAttachment>,
        thread_guid: &str,
        threading: Threading,
    ) {
        let mut parent_thread = self.lookup_thread(thread_guid, channel_id);
        let mut start_with_msg = false;

        if parent_thread.is_none() {
            match threading {
                Threading::ReplyOrStart(_, root) => {
                    parent_thread = self
                        .start_thread(channel_id, channel_name, root, thread_guid)
                        .await;
                }
                Threading::Root(_) => start_with_msg = true,
                Threading::Reply(_) | Threading::Flat => (),
            }
        }

        let slack_msg = SlackMessage {
            text: msg.to_string(),
            attachments,
            channel: channel_id.to_string(),
            thread_ts: parent_thread.clone(),
        };

        match self.post_unique(&slack_msg, channel_name).await {
            Posted::Sent(_) if parent_thread.is_some() => {
                self.touch_thread(thread_guid, channel_id);
            }
            Posted::Sent(thread) if start_with_msg => {
                self.save_thread(thread_guid, channel_id, &thread);
            }
            Posted::Rejected if parent_thread.is_some() => {
                // A thread Slack won't reply to shouldn't swallow the message.
                let top_level = SlackMessage {
                    thread_ts: None,
                    ..slack_msg
                };
                self.post(&top_level, channel_name).await;
            }
            _ => (),
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
        let Posted::Sent(thread) = self.post(&root_msg, channel_name).await else {
            return None;
        };
        self.save_thread(thread_guid, channel_id, &thread);
        Some(thread)
    }

    fn save_thread(&self, thread_guid: &str, channel_id: &str, thread: &str) {
        let slack_db = self.slack_db.lock().unwrap();
        if let Err(e) = slack_db.insert_thread(thread_guid, channel_id, thread) {
            error!("{}", e);
        }
    }

    fn touch_thread(&self, thread_guid: &str, channel_id: &str) {
        let slack_db = self.slack_db.lock().unwrap();
        if let Err(e) = slack_db.touch_thread(thread_guid, channel_id) {
            error!("{}", e);
        }
    }

    async fn post_unique(&self, slack_msg: &SlackMessage, channel_name: &str) -> Posted {
        if !self.is_unique(slack_msg) {
            info!("Skipping duplicate message to {}", channel_name);
            return Posted::Failed;
        }
        self.post(slack_msg, channel_name).await
    }

    async fn post(&self, slack_msg: &SlackMessage, channel_name: &str) -> Posted {
        debug!("Sending message to #{}", channel_name);

        let res: Result<SlackResponse> = self.client.post("/chat.postMessage", slack_msg).await;
        match res {
            Ok(r) if r.ok => {
                let thread = r.ts.unwrap_or_default();
                info!(
                    "Successfully sent slack message to {}, ts: \"{}\"",
                    channel_name, thread
                );
                Posted::Sent(thread)
            }
            Ok(r) => {
                error!(
                    "Error sending slack message to {}: {} ({})",
                    channel_name,
                    slack_msg.channel,
                    r.error.unwrap_or_default(),
                );
                Posted::Rejected
            }
            Err(e) => {
                error!(
                    "Error sending slack message to {} ({}): {}",
                    channel_name, slack_msg.channel, e
                );
                Posted::Failed
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
    pub msg: String,
    pub attachments: Vec<SlackAttachment>,
    pub threading: Threading,
}

// How a message joins the thread stored for a (guid, channel) pair.
#[derive(Debug, PartialEq, Clone)]
pub enum Threading {
    Flat,
    // Reply if the thread exists, otherwise post top-level without starting one.
    Reply(String),
    // Reply if the thread exists, otherwise this message starts it.
    Root(String),
    // Reply if the thread exists, otherwise post the root, start the thread with it, and reply.
    ReplyOrStart(String, ThreadRoot),
}

impl Threading {
    pub fn guid(&self) -> Option<&str> {
        match self {
            Threading::Flat => None,
            Threading::Reply(guid) | Threading::Root(guid) | Threading::ReplyOrStart(guid, _) => {
                Some(guid)
            }
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
    threading: Threading,
) -> SlackRequest {
    SlackRequest {
        channel,
        msg: msg.into(),
        attachments: attachments.into(),
        threading,
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
                req.threading,
            )
            .await;
    }
}

#[cfg(test)]
mod tests {
    use mockito::{Matcher, Mock, Server};
    use serde_json::json;
    use tempfile::{TempDir, tempdir};

    use super::*;

    const GUID: &str = "org/repo/1";
    const CHANNEL: &str = "C123";

    fn new_test(server: &Server) -> (Slack, TempDir) {
        let dir = tempdir().unwrap();
        let db = dir.path().join("slack.sqlite3");
        let slack = Slack::new_with_url(
            &server.url(),
            "token".into(),
            db.to_string_lossy().into(),
            Metrics::new(),
        );
        (slack, dir)
    }

    async fn expect_post(server: &mut Server, body: serde_json::Value, resp: &str) -> Mock {
        server
            .mock("POST", "/chat.postMessage")
            .match_body(Matcher::PartialJson(body))
            .with_body(resp)
            .expect(1)
            .create_async()
            .await
    }

    fn root() -> ThreadRoot {
        ThreadRoot {
            msg: "root".into(),
            attachments: vec![],
        }
    }

    fn saved_thread(slack: &Slack) -> Option<String> {
        let db = slack.slack_db.lock().unwrap();
        db.lookup_previous_thread(GUID, CHANNEL).unwrap()
    }

    #[tokio::test]
    async fn test_reply_or_start_posts_root_then_reply() {
        let mut server = Server::new_async().await;
        let (slack, _dir) = new_test(&server);
        let root_post = expect_post(
            &mut server,
            json!({"text": "root", "thread_ts": null}),
            r#"{"ok": true, "ts": "1.0"}"#,
        )
        .await;
        let reply = expect_post(
            &mut server,
            json!({"text": "hello", "thread_ts": "1.0"}),
            r#"{"ok": true, "ts": "2.0"}"#,
        )
        .await;

        let threading = Threading::ReplyOrStart(GUID.into(), root());
        slack
            .send(CHANNEL, "chan", "hello", vec![], threading)
            .await;

        root_post.assert_async().await;
        reply.assert_async().await;
        assert_eq!(Some("1.0".to_string()), saved_thread(&slack));
    }

    #[tokio::test]
    async fn test_reply_uses_existing_thread() {
        let mut server = Server::new_async().await;
        let (slack, _dir) = new_test(&server);
        slack.save_thread(GUID, CHANNEL, "1.0");
        let reply = expect_post(
            &mut server,
            json!({"text": "hello", "thread_ts": "1.0"}),
            r#"{"ok": true, "ts": "2.0"}"#,
        )
        .await;

        let threading = Threading::ReplyOrStart(GUID.into(), root());
        slack
            .send(CHANNEL, "chan", "hello", vec![], threading)
            .await;

        reply.assert_async().await;
    }

    #[tokio::test]
    async fn test_root_starts_thread_with_message() {
        let mut server = Server::new_async().await;
        let (slack, _dir) = new_test(&server);
        let post = expect_post(
            &mut server,
            json!({"text": "hello", "thread_ts": null}),
            r#"{"ok": true, "ts": "3.0"}"#,
        )
        .await;

        let threading = Threading::Root(GUID.into());
        slack
            .send(CHANNEL, "chan", "hello", vec![], threading)
            .await;

        post.assert_async().await;
        assert_eq!(Some("3.0".to_string()), saved_thread(&slack));
    }

    #[tokio::test]
    async fn test_reply_without_thread_stays_top_level() {
        let mut server = Server::new_async().await;
        let (slack, _dir) = new_test(&server);
        let post = expect_post(
            &mut server,
            json!({"text": "hello", "thread_ts": null}),
            r#"{"ok": true, "ts": "3.0"}"#,
        )
        .await;

        let threading = Threading::Reply(GUID.into());
        slack
            .send(CHANNEL, "chan", "hello", vec![], threading)
            .await;

        post.assert_async().await;
        assert_eq!(None, saved_thread(&slack));
    }

    #[tokio::test]
    async fn test_concurrent_sends_share_one_thread() {
        let mut server = Server::new_async().await;
        let (slack, _dir) = new_test(&server);
        let root_post = expect_post(
            &mut server,
            json!({"text": "root", "thread_ts": null}),
            r#"{"ok": true, "ts": "1.0"}"#,
        )
        .await;
        let first = expect_post(
            &mut server,
            json!({"text": "one", "thread_ts": "1.0"}),
            r#"{"ok": true, "ts": "2.0"}"#,
        )
        .await;
        let second = expect_post(
            &mut server,
            json!({"text": "two", "thread_ts": "1.0"}),
            r#"{"ok": true, "ts": "3.0"}"#,
        )
        .await;

        tokio::join!(
            slack.send(
                CHANNEL,
                "chan",
                "one",
                vec![],
                Threading::ReplyOrStart(GUID.into(), root())
            ),
            slack.send(
                CHANNEL,
                "chan",
                "two",
                vec![],
                Threading::ReplyOrStart(GUID.into(), root())
            ),
        );

        root_post.assert_async().await;
        first.assert_async().await;
        second.assert_async().await;
        assert!(slack.thread_locks.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_rejected_reply_falls_back_to_top_level() {
        let mut server = Server::new_async().await;
        let (slack, _dir) = new_test(&server);
        slack.save_thread(GUID, CHANNEL, "1.0");
        let reply = expect_post(
            &mut server,
            json!({"text": "hello", "thread_ts": "1.0"}),
            r#"{"ok": false, "error": "thread_not_found"}"#,
        )
        .await;
        let top_level = expect_post(
            &mut server,
            json!({"text": "hello", "thread_ts": null}),
            r#"{"ok": true, "ts": "2.0"}"#,
        )
        .await;

        let threading = Threading::Reply(GUID.into());
        slack
            .send(CHANNEL, "chan", "hello", vec![], threading)
            .await;

        reply.assert_async().await;
        top_level.assert_async().await;
    }
}
