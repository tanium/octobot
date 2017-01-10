
pub trait Messenger: Send + Sync {
    fn send_to_all(&self);

    fn send_to_channel(&self);

    fn send_to_owner(&self);
}

#[derive(Clone)]
pub struct SlackMessenger {
    pub slack_webhook_url: String,
}

unsafe impl Send for SlackMessenger {}
unsafe impl Sync for SlackMessenger {}


impl Messenger for SlackMessenger {
    fn send_to_all(&self) {}

    fn send_to_channel(&self) {}

    fn send_to_owner(&self) {}
}
