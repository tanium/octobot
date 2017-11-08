use std::collections::HashMap;
use std::sync::Mutex;
use std::thread;

use octobot::errors::*;
use octobot::jira::*;
use octobot::jira::api::{JiraVersionPosition, Session};
use octobot::version;

pub struct MockJira {
    get_transitions_calls: Mutex<Vec<MockCall<Vec<Transition>>>>,
    transition_issue_calls: Mutex<Vec<MockCall<()>>>,
    comment_issue_calls: Mutex<Vec<MockCall<()>>>,
    add_version_calls: Mutex<Vec<MockCall<()>>>,
    get_versions_calls: Mutex<Vec<MockCall<Vec<Version>>>>,
    assign_fix_version_calls: Mutex<Vec<MockCall<()>>>,
    reorder_version_calls: Mutex<Vec<MockCall<()>>>,
    add_pending_version_calls: Mutex<Vec<MockCall<()>>>,
    remove_pending_versions_calls: Mutex<Vec<MockCall<()>>>,
    find_pending_versions_calls: Mutex<Vec<MockCall<HashMap<String, Vec<version::Version>>>>>,
}

#[derive(Debug)]
struct MockCall<T> {
    args: Vec<String>,
    ret: Result<T>,
}

impl<T> MockCall<T> {
    pub fn new(ret: Result<T>, args: Vec<&str>) -> MockCall<T> {
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
            get_versions_calls: Mutex::new(vec![]),
            assign_fix_version_calls: Mutex::new(vec![]),
            reorder_version_calls: Mutex::new(vec![]),
            add_pending_version_calls: Mutex::new(vec![]),
            remove_pending_versions_calls: Mutex::new(vec![]),
            find_pending_versions_calls: Mutex::new(vec![]),
        }
    }
}

impl Drop for MockJira {
    fn drop(&mut self) {
        if !thread::panicking() {
            assert!(
                self.get_transitions_calls.lock().unwrap().len() == 0,
                "Unmet get_transitions calls: {:?}",
                *self.get_transitions_calls.lock().unwrap()
            );
            assert!(
                self.transition_issue_calls.lock().unwrap().len() == 0,
                "Unmet transition_issue calls: {:?}",
                *self.transition_issue_calls.lock().unwrap()
            );
            assert!(
                self.comment_issue_calls.lock().unwrap().len() == 0,
                "Unmet comment_issue calls: {:?}",
                *self.comment_issue_calls.lock().unwrap()
            );
            assert!(
                self.add_version_calls.lock().unwrap().len() == 0,
                "Unmet add_version calls: {:?}",
                *self.add_version_calls.lock().unwrap()
            );
            assert!(
                self.get_versions_calls.lock().unwrap().len() == 0,
                "Unmet get_versions calls: {:?}",
                *self.get_versions_calls.lock().unwrap()
            );
            assert!(
                self.assign_fix_version_calls.lock().unwrap().len() == 0,
                "Unmet asign_fix_version calls: {:?}",
                *self.assign_fix_version_calls.lock().unwrap()
            );
            assert!(
                self.reorder_version_calls.lock().unwrap().len() == 0,
                "Unmet reorder_version calls: {:?}",
                *self.reorder_version_calls.lock().unwrap()
            );
            assert!(
                self.add_pending_version_calls.lock().unwrap().len() == 0,
                "Unmet add_pending_version calls: {:?}",
                *self.add_pending_version_calls.lock().unwrap()
            );
            assert!(
                self.remove_pending_versions_calls.lock().unwrap().len() == 0,
                "Unmet remove_pending_versions calls: {:?}",
                *self.remove_pending_versions_calls.lock().unwrap()
            );
            assert!(
                self.find_pending_versions_calls.lock().unwrap().len() == 0,
                "Unmet find_pending_versions calls: {:?}",
                *self.find_pending_versions_calls.lock().unwrap()
            );
        }
    }
}

impl Session for MockJira {
    fn get_transitions(&self, key: &str) -> Result<Vec<Transition>> {
        let mut calls = self.get_transitions_calls.lock().unwrap();
        assert!(calls.len() > 0, "Unexpected call to get_transitions");
        let call = calls.remove(0);
        assert_eq!(call.args[0], key);

        call.ret
    }

    fn transition_issue(&self, key: &str, req: &TransitionRequest) -> Result<()> {
        let mut calls = self.transition_issue_calls.lock().unwrap();
        assert!(calls.len() > 0, "Unexpected call to transition_issue");
        let call = calls.remove(0);
        assert_eq!(call.args[0], key);
        assert_eq!(call.args[1], format!("{:?}", req));

        call.ret
    }

    fn comment_issue(&self, key: &str, comment: &str) -> Result<()> {
        let mut calls = self.comment_issue_calls.lock().unwrap();
        assert!(calls.len() > 0, "Unexpected call to comment_issue");
        let call = calls.remove(0);
        assert_eq!(call.args[0], key);
        assert_eq!(call.args[1], comment);

        call.ret
    }

    fn add_version(&self, proj: &str, version: &str) -> Result<()> {
        let mut calls = self.add_version_calls.lock().unwrap();
        assert!(calls.len() > 0, "Unexpected call to add_version");
        let call = calls.remove(0);
        assert_eq!(call.args[0], proj);
        assert_eq!(call.args[1], version);

        call.ret
    }

    fn get_versions(&self, proj: &str) -> Result<Vec<Version>> {
        let mut calls = self.get_versions_calls.lock().unwrap();
        assert!(calls.len() > 0, "Unexpected call to get_versions");
        let call = calls.remove(0);
        assert_eq!(call.args[0], proj);

        call.ret
    }

    fn assign_fix_version(&self, key: &str, version: &str) -> Result<()> {
        let mut calls = self.assign_fix_version_calls.lock().unwrap();
        assert!(calls.len() > 0, "Unexpected call to assign_fix_version");
        let call = calls.remove(0);
        assert_eq!(call.args[0], key);
        assert_eq!(call.args[1], version);

        call.ret
    }

    fn reorder_version(&self, version: &Version, position: JiraVersionPosition) -> Result<()> {
        let mut calls = self.reorder_version_calls.lock().unwrap();
        assert!(calls.len() > 0, "Unexpected call to reorder_version");
        let call = calls.remove(0);
        assert_eq!(call.args[0], format!("{:?}", version));
        assert_eq!(call.args[1], format!("{:?}", position));

        call.ret
    }

    fn add_pending_version(&self, key: &str, version: &str) -> Result<()> {
        let mut calls = self.add_pending_version_calls.lock().unwrap();
        assert!(calls.len() > 0, "Unexpected call to add_pending_version");
        let call = calls.remove(0);
        assert_eq!(call.args[0], key);
        assert_eq!(call.args[1], version);

        call.ret
    }

    fn remove_pending_versions(&self, key: &str, versions: &Vec<version::Version>) -> Result<()> {
        let mut calls = self.remove_pending_versions_calls.lock().unwrap();
        assert!(calls.len() > 0, "Unexpected call to remove_pending_versions");
        let call = calls.remove(0);
        assert_eq!(call.args[0], key);
        assert_eq!(call.args[1], format!("{:?}", versions));

        call.ret
    }

    fn find_pending_versions(&self, proj: &str) -> Result<HashMap<String, Vec<version::Version>>> {
        let mut calls = self.find_pending_versions_calls.lock().unwrap();
        assert!(calls.len() > 0, "Unexpected call to find_pending_versions");
        let call = calls.remove(0);
        assert_eq!(call.args[0], proj);

        call.ret
    }
}

impl MockJira {
    pub fn mock_get_transitions(&self, key: &str, ret: Result<Vec<Transition>>) {
        self.get_transitions_calls.lock().unwrap().push(MockCall::new(ret, vec![key]));
    }

    pub fn mock_transition_issue(&self, key: &str, req: &TransitionRequest, ret: Result<()>) {
        self.transition_issue_calls.lock().unwrap().push(MockCall::new(
            ret,
            vec![key, &format!("{:?}", req)],
        ));
    }

    pub fn mock_comment_issue(&self, key: &str, comment: &str, ret: Result<()>) {
        self.comment_issue_calls.lock().unwrap().push(MockCall::new(ret, vec![key, comment]));
    }

    pub fn mock_add_version(&self, proj: &str, version: &str, ret: Result<()>) {
        self.add_version_calls.lock().unwrap().push(MockCall::new(ret, vec![proj, version]));
    }

    pub fn mock_get_versions(&self, proj: &str, ret: Result<Vec<Version>>) {
        self.get_versions_calls.lock().unwrap().push(MockCall::new(ret, vec![proj]));
    }

    pub fn mock_assign_fix_version(&self, key: &str, version: &str, ret: Result<()>) {
        self.assign_fix_version_calls.lock().unwrap().push(MockCall::new(ret, vec![key, version]));
    }

    pub fn mock_reorder_version(&self, version: &Version, position: JiraVersionPosition, ret: Result<()>) {
        self.reorder_version_calls.lock().unwrap().push(MockCall::new(
            ret,
            vec![
                &format!("{:?}", version),
                &format!("{:?}", position),
            ],
        ));
    }

    pub fn mock_add_pending_version(&self, key: &str, version: &str, ret: Result<()>) {
        self.add_pending_version_calls.lock().unwrap().push(
            MockCall::new(ret, vec![key, version]),
        );
    }

    pub fn mock_remove_pending_versions(&self, key: &str, versions: &Vec<version::Version>, ret: Result<()>) {
        self.remove_pending_versions_calls.lock().unwrap().push(MockCall::new(
            ret,
            vec![key, &format!("{:?}", versions)],
        ));
    }

    pub fn mock_find_pending_versions(&self, proj: &str, ret: Result<HashMap<String, Vec<version::Version>>>) {
        self.find_pending_versions_calls.lock().unwrap().push(MockCall::new(ret, vec![proj]));
    }
}
