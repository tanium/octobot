use http_client::HTTPClient;

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

        let client = HTTPClient::new(&self.webhook_url)
            .with_headers(hashmap!{
                "Content-Type" => "application/json".to_string(),
            });

        client.post_void("", &slack_msg)
    }
}
