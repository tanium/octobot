use std::sync::Mutex;
use std::thread;

use octobot::jira::*;
use octobot::jira::api::Session;

pub struct MockJira {
    get_transitions_calls: Mutex<Vec< MockCall<Vec<Transition>> >>,
    transition_issue_calls: Mutex<Vec< MockCall<()> >>,
    comment_issue_calls: Mutex<Vec< MockCall<()> >>,
}

#[derive(Debug)]
struct MockCall<T> {
    args: Vec<String>,
    ret: Result<T, String>,
}

impl<T> MockCall<T> {
    pub fn new(ret: Result<T, String>, args: Vec<&str>) -> MockCall<T> {
        MockCall {
            ret: ret,
            args: args.iter().map(|a| a.to_string()).collect(),
        }
    }
}

impl MockJira {
    pub fn new() -> MockJira {
        MockJira {
            get_transitions_calls: Mutex::new(vec![]),
            transition_issue_calls: Mutex::new(vec![]),
            comment_issue_calls: Mutex::new(vec![]),
        }
    }
}

impl Drop for MockJira {
    fn drop(&mut self) {
        if !thread::panicking() {
            if self.get_transitions_calls.lock().unwrap().len() > 0 {
                panic!("Unmet get_transitions calls: {:?}", *self.get_transitions_calls.lock().unwrap());
            }
            if self.transition_issue_calls.lock().unwrap().len() > 0 {
                panic!("Unmet transition_issue calls: {:?}", *self.transition_issue_calls.lock().unwrap());
            }
            if self.comment_issue_calls.lock().unwrap().len() > 0 {
                panic!("Unmet comment_issue calls: {:?}", *self.comment_issue_calls.lock().unwrap());
            }
        }
    }
}

impl Session for MockJira {
    fn get_transitions(&self, key: &str) -> Result<Vec<Transition>, String> {
        let mut calls = self.get_transitions_calls.lock().unwrap();
        if calls.len() == 0 {
            panic!("Unexpected call to get_transitions");
        }
        let call = calls.remove(0);
        assert_eq!(call.args[0], key);

        call.ret
    }

    fn transition_issue(&self, key: &str, req: &TransitionRequest) -> Result<(), String> {
        let mut calls = self.transition_issue_calls.lock().unwrap();
        if calls.len() == 0 {
            panic!("Unexpected call to transition_issue");
        }
        let call = calls.remove(0);
        assert_eq!(call.args[0], key);
        assert_eq!(call.args[1], format!("{:?}", req));

        call.ret
    }

    fn comment_issue(&self, key: &str, comment: &str) -> Result<(), String> {
        let mut calls = self.comment_issue_calls.lock().unwrap();
        if calls.len() == 0 {
            panic!("Unexpected call to comment_issue");
        }
        let call = calls.remove(0);
        assert_eq!(call.args[0], key);
        assert_eq!(call.args[1], comment);

        call.ret
    }
}

impl MockJira {
    pub fn mock_get_transitions(&self, key: &str, ret: Result<Vec<Transition>, String>) {
        self.get_transitions_calls.lock().unwrap().push(MockCall::new(ret, vec![key]));
    }

    pub fn mock_transition_issue(&self, key: &str, req: &TransitionRequest, ret: Result<(), String>) {
        self.transition_issue_calls.lock().unwrap().push(MockCall::new(ret, vec![key, &format!("{:?}", req)]));
    }

    pub fn mock_comment_issue(&self, key: &str, comment: &str, ret: Result<(), String>) {
        self.comment_issue_calls.lock().unwrap().push(MockCall::new(ret, vec![key, comment]));
    }
}
