use std::sync::Mutex;
use std::thread;

use octobot::jira::*;
use octobot::jira::api::Session;

pub struct MockJira {
    get_transitions_calls: Mutex<Vec< MockCall<Vec<Transition>> >>,
    transition_issue_calls: Mutex<Vec< MockCall<()> >>,
    comment_issue_calls: Mutex<Vec< MockCall<()> >>,
    add_version_calls: Mutex<Vec< MockCall<()> >>,
    assign_fix_version_calls: Mutex<Vec< MockCall<()> >>,
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
            add_version_calls: Mutex::new(vec![]),
            assign_fix_version_calls: Mutex::new(vec![]),
        }
    }
}

impl Drop for MockJira {
    fn drop(&mut self) {
        if !thread::panicking() {
            assert!(self.get_transitions_calls.lock().unwrap().len() == 0,
                    "Unmet get_transitions calls: {:?}", *self.get_transitions_calls.lock().unwrap());
            assert!(self.transition_issue_calls.lock().unwrap().len() == 0,
                    "Unmet transition_issue calls: {:?}", *self.transition_issue_calls.lock().unwrap());
            assert!(self.comment_issue_calls.lock().unwrap().len() == 0,
                    "Unmet comment_issue calls: {:?}", *self.comment_issue_calls.lock().unwrap());
            assert!(self.add_version_calls.lock().unwrap().len() == 0,
                    "Unmet add_version calls: {:?}", *self.add_version_calls.lock().unwrap());
            assert!(self.assign_fix_version_calls.lock().unwrap().len() == 0,
                    "Unmet asign_fix_version calls: {:?}", *self.assign_fix_version_calls.lock().unwrap());

        }
    }
}

impl Session for MockJira {
    fn get_transitions(&self, key: &str) -> Result<Vec<Transition>, String> {
        let mut calls = self.get_transitions_calls.lock().unwrap();
        assert!(calls.len() > 0, "Unexpected call to get_transitions");
        let call = calls.remove(0);
        assert_eq!(call.args[0], key);

        call.ret
    }

    fn transition_issue(&self, key: &str, req: &TransitionRequest) -> Result<(), String> {
        let mut calls = self.transition_issue_calls.lock().unwrap();
        assert!(calls.len() > 0, "Unexpected call to transition_issue");
        let call = calls.remove(0);
        assert_eq!(call.args[0], key);
        assert_eq!(call.args[1], format!("{:?}", req));

        call.ret
    }

    fn comment_issue(&self, key: &str, comment: &str) -> Result<(), String> {
        let mut calls = self.comment_issue_calls.lock().unwrap();
        assert!(calls.len() > 0, "Unexpected call to comment_issue");
        let call = calls.remove(0);
        assert_eq!(call.args[0], key);
        assert_eq!(call.args[1], comment);

        call.ret
    }

    fn add_version(&self, proj: &str, version: &str) -> Result<(), String> {
        let mut calls = self.add_version_calls.lock().unwrap();
        assert!(calls.len() > 0, "Unexpected call to add_version");
        let call = calls.remove(0);
        assert_eq!(call.args[0], proj);
        assert_eq!(call.args[1], version);

        call.ret
    }

    fn assign_fix_version(&self, key: &str, version: &str) -> Result<(), String> {
        let mut calls = self.assign_fix_version_calls.lock().unwrap();
        assert!(calls.len() > 0, "Unexpected call to assign_fix_version");
        let call = calls.remove(0);
        assert_eq!(call.args[0], key);
        assert_eq!(call.args[1], version);

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

    pub fn mock_add_version(&self, proj: &str, version: &str, ret: Result<(), String>) {
        self.add_version_calls.lock().unwrap().push(MockCall::new(ret, vec![proj, version]));
    }

    pub fn mock_assign_fix_version(&self, key: &str, version: &str, ret: Result<(), String>) {
        self.assign_fix_version_calls.lock().unwrap().push(MockCall::new(ret, vec![key, version]));
    }
}
