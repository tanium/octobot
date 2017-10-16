use std::sync::{Arc, Mutex};
use std::sync::mpsc::channel;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use octobot::slack::SlackRequest;
use octobot::worker::{WorkMessage, WorkSender};

pub struct MockSlack {
    expected_calls: Arc<Mutex<Vec<SlackRequest>>>,
    slack_sender: WorkSender<SlackRequest>,
    thread: Option<JoinHandle<()>>,
}

impl MockSlack {
    pub fn new(mut expected_calls: Vec<SlackRequest>) -> MockSlack {
        let (slack_tx, slack_rx) = channel();

        // reverse them so we can pop fron the back
        expected_calls.reverse();

        let expected_calls = Arc::new(Mutex::new(expected_calls));
        let expected_calls2 = expected_calls.clone();

        let thread = Some(thread::spawn(move || {
            let timeout = Duration::from_millis(1000);
            loop {
                let req = slack_rx.recv_timeout(timeout);
                match req {
                    Ok(WorkMessage::WorkItem(req)) => {
                        let front = expected_calls2.lock().unwrap().pop();
                        match front {
                            Some(call) => assert_eq!(call, req),
                            None => panic!("Unexpected message: {:?}", req),
                        }
                    }
                    Ok(WorkMessage::Stop) => break,
                    Err(_) => {
                        let is_empty = expected_calls2.lock().unwrap().is_empty();
                        if !is_empty {
                            panic!("No messages received, but expected more");
                        }
                    }
                };
            }
        }));

        MockSlack {
            expected_calls: expected_calls,
            slack_sender: WorkSender::new(slack_tx),
            thread: thread,
        }
    }

    pub fn expect(&mut self, mut calls: Vec<SlackRequest>) {
        // reverse them so we can pop fron the back
        calls.reverse();

        *self.expected_calls.lock().unwrap() = calls;
    }

    pub fn new_sender(&self) -> WorkSender<SlackRequest> {
        self.slack_sender.clone()
    }
}

impl Drop for MockSlack {
    fn drop(&mut self) {
        if !thread::panicking() {
            self.slack_sender.stop().expect("failed to stop slack");
            self.thread.take().unwrap().join().expect("failed to wait for thread");
            assert!(
                self.expected_calls.lock().unwrap().is_empty(),
                "Failed: Still expecting calls: {:?}",
                *self.expected_calls.lock().unwrap()
            )
        }
    }
}
