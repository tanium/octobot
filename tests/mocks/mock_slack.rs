use std::cell::Cell;

use octobot::slack::{SlackSender, SlackAttachment};

pub struct SlackCall {
    pub channel: String,
    pub msg: String,
    pub attachments: Vec<SlackAttachment>,
}

impl SlackCall {
    pub fn new(channel: &str, msg: &str, attach: Vec<SlackAttachment>) -> SlackCall {
        SlackCall {
            channel: channel.to_string(),
            msg: msg.to_string(),
            attachments: attach,
        }
    }
}

pub struct MockSlack {
    pub expected_calls: Vec<SlackCall>,
    pub call_count: Cell<usize>,
    pub should_fail: Option<String>,
}

impl MockSlack {
    pub fn new(calls: Vec<SlackCall>) -> MockSlack {
        MockSlack {
            should_fail: None,
            call_count: Cell::new(0),
            expected_calls: calls,
        }
    }

    pub fn expect(&mut self, calls: Vec<SlackCall>) {
        self.expected_calls = calls;
    }
}

impl SlackSender for MockSlack {
    fn send(&self, channel: &str, msg: &str, attachments: Vec<SlackAttachment>)
            -> Result<(), String> {

        if self.call_count.get() >= self.expected_calls.len() {
            panic!("Failed: received unexpected slack call: ({}, {}, {:?})",
                   channel,
                   msg,
                   attachments);
        }

        assert_eq!(self.expected_calls[self.call_count.get()].channel, channel);
        assert_eq!(self.expected_calls[self.call_count.get()].msg, msg);
        assert_eq!(self.expected_calls[self.call_count.get()].attachments,
                   attachments);
        self.call_count.set(self.call_count.get() + 1);

        if let Some(ref msg) = self.should_fail {
            Err(msg.clone())
        } else {
            Ok(())
        }
    }
}

impl Drop for MockSlack {
    fn drop(&mut self) {
        if self.call_count.get() != self.expected_calls.len() {
            println!("Failed: Expected {} calls but only received {}",
                     self.expected_calls.len(),
                     self.call_count.get());
        }
    }
}
