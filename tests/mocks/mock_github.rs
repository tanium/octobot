use std::sync::Mutex;
use std::thread;

use octobot::errors::*;
use octobot::github::*;
use octobot::github::api::Session;

pub struct MockGithub {
    user: User,
    host: String,
    token: String,

    get_pr_calls: Mutex<Vec<MockCall<PullRequest>>>,
    get_prs_calls: Mutex<Vec<MockCall<Vec<PullRequest>>>>,
    create_pr_calls: Mutex<Vec<MockCall<PullRequest>>>,
    get_pr_labels_calls: Mutex<Vec<MockCall<Vec<Label>>>>,
    get_pr_commits_calls: Mutex<Vec<MockCall<Vec<Commit>>>>,
    get_pr_reviews_calls: Mutex<Vec<MockCall<Vec<Review>>>>,
    assign_pr_calls: Mutex<Vec<MockCall<AssignResponse>>>,
    comment_pr_calls: Mutex<Vec<MockCall<()>>>,
    create_branch_calls: Mutex<Vec<MockCall<()>>>,
    delete_branch_calls: Mutex<Vec<MockCall<()>>>,
    get_status_calls: Mutex<Vec<MockCall<Vec<Status>>>>,
    create_status_calls: Mutex<Vec<MockCall<()>>>,
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

impl MockGithub {
    pub fn new() -> MockGithub {
        MockGithub {
            user: User::new("octobot"),
            host: "the-github-host".to_string(),
            token: "the-github-token".to_string(),

            get_pr_calls: Mutex::new(vec![]),
            get_prs_calls: Mutex::new(vec![]),
            create_pr_calls: Mutex::new(vec![]),
            get_pr_labels_calls: Mutex::new(vec![]),
            get_pr_commits_calls: Mutex::new(vec![]),
            get_pr_reviews_calls: Mutex::new(vec![]),
            assign_pr_calls: Mutex::new(vec![]),
            comment_pr_calls: Mutex::new(vec![]),
            create_branch_calls: Mutex::new(vec![]),
            delete_branch_calls: Mutex::new(vec![]),
            get_status_calls: Mutex::new(vec![]),
            create_status_calls: Mutex::new(vec![]),
        }
    }
}

impl Drop for MockGithub {
    fn drop(&mut self) {
        if !thread::panicking() {
            assert!(
                self.get_pr_calls.lock().unwrap().len() == 0,
                "Unmet get_pull_request calls: {:?}",
                *self.get_pr_calls.lock().unwrap()
            );
            assert!(
                self.get_prs_calls.lock().unwrap().len() == 0,
                "Unmet get_pull_requests calls: {:?}",
                *self.get_prs_calls.lock().unwrap()
            );
            assert!(
                self.create_pr_calls.lock().unwrap().len() == 0,
                "Unmet create_pull_request calls: {:?}",
                *self.create_pr_calls.lock().unwrap()
            );
            assert!(
                self.get_pr_labels_calls.lock().unwrap().len() == 0,
                "Unmet get_pull_request_labels calls: {:?}",
                *self.get_pr_labels_calls.lock().unwrap()
            );
            assert!(
                self.assign_pr_calls.lock().unwrap().len() == 0,
                "Unmet assign_pull_request calls: {:?}",
                *self.assign_pr_calls.lock().unwrap()
            );
            assert!(
                self.comment_pr_calls.lock().unwrap().len() == 0,
                "Unmet comment_pull_request calls: {:?}",
                *self.comment_pr_calls.lock().unwrap()
            );
            assert!(
                self.create_branch_calls.lock().unwrap().len() == 0,
                "Unmet create_branch calls: {:?}",
                *self.create_branch_calls.lock().unwrap()
            );
            assert!(
                self.delete_branch_calls.lock().unwrap().len() == 0,
                "Unmet delete_branch calls: {:?}",
                *self.delete_branch_calls.lock().unwrap()
            );
        }
    }
}

impl Session for MockGithub {
    fn user(&self) -> &User {
        &self.user
    }

    fn github_host(&self) -> &str {
        &self.host
    }

    fn github_token(&self) -> &str {
        &self.token
    }

    fn get_pull_request(&self, owner: &str, repo: &str, number: u32) -> Result<PullRequest> {
        let mut calls = self.get_pr_calls.lock().unwrap();
        if calls.len() == 0 {
            panic!("Unexpected call to get_pull_request");
        }
        let call = calls.remove(0);
        assert_eq!(call.args[0], owner);
        assert_eq!(call.args[1], repo);
        assert_eq!(call.args[2], number.to_string());

        call.ret
    }

    fn get_pull_requests(
        &self,
        owner: &str,
        repo: &str,
        state: Option<&str>,
        head: Option<&str>,
    ) -> Result<Vec<PullRequest>> {
        let mut calls = self.get_prs_calls.lock().unwrap();
        assert!(calls.len() > 0, "Unexpected call to get_pull_requests");
        let call = calls.remove(0);
        assert_eq!(call.args[0], owner);
        assert_eq!(call.args[1], repo);
        assert_eq!(call.args[2], state.unwrap_or(""));
        assert_eq!(call.args[3], head.unwrap_or(""));

        call.ret
    }

    fn create_pull_request(
        &self,
        owner: &str,
        repo: &str,
        title: &str,
        body: &str,
        head: &str,
        base: &str,
    ) -> Result<PullRequest> {
        let mut calls = self.create_pr_calls.lock().unwrap();
        assert!(calls.len() > 0, "Unexpected call to create_pull_request");
        let call = calls.remove(0);
        assert_eq!(call.args[0], owner);
        assert_eq!(call.args[1], repo);
        assert_eq!(call.args[2], title);
        assert_eq!(call.args[3], body);
        assert_eq!(call.args[4], head);
        assert_eq!(call.args[5], base);

        call.ret
    }

    fn get_pull_request_labels(&self, owner: &str, repo: &str, number: u32) -> Result<Vec<Label>> {
        let mut calls = self.get_pr_labels_calls.lock().unwrap();
        assert!(calls.len() > 0, "Unexpected call to get_pull_request_labels");
        let call = calls.remove(0);
        assert_eq!(call.args[0], owner);
        assert_eq!(call.args[1], repo);
        assert_eq!(call.args[2], number.to_string());

        call.ret
    }

    fn get_pull_request_commits(&self, owner: &str, repo: &str, number: u32) -> Result<Vec<Commit>> {
        let mut calls = self.get_pr_commits_calls.lock().unwrap();
        assert!(calls.len() > 0, "Unexpected call to get_pull_request_commits");
        let call = calls.remove(0);
        assert_eq!(call.args[0], owner);
        assert_eq!(call.args[1], repo);
        assert_eq!(call.args[2], number.to_string());

        call.ret
    }

    fn get_pull_request_reviews(&self, owner: &str, repo: &str, number: u32) -> Result<Vec<Review>> {
        let mut calls = self.get_pr_reviews_calls.lock().unwrap();
        assert!(calls.len() > 0, "Unexpected call to get_pull_request_reviews");
        let call = calls.remove(0);
        assert_eq!(call.args[0], owner);
        assert_eq!(call.args[1], repo);
        assert_eq!(call.args[2], number.to_string());

        call.ret
    }

    fn assign_pull_request(
        &self,
        owner: &str,
        repo: &str,
        number: u32,
        assignees: Vec<String>,
    ) -> Result<AssignResponse> {
        let mut calls = self.assign_pr_calls.lock().unwrap();
        assert!(calls.len() > 0, "Unexpected call to assign_pull_request");
        let call = calls.remove(0);
        assert_eq!(call.args[0], owner);
        assert_eq!(call.args[1], repo);
        assert_eq!(call.args[2], number.to_string());
        assert_eq!(call.args[3], assignees.join(","));

        call.ret
    }

    fn comment_pull_request(&self, owner: &str, repo: &str, number: u32, comment: &str) -> Result<()> {
        let mut calls = self.comment_pr_calls.lock().unwrap();
        assert!(calls.len() > 0, "Unexpected call to comment_pull_request");
        let call = calls.remove(0);
        assert_eq!(call.args[0], owner);
        assert_eq!(call.args[1], repo);
        assert_eq!(call.args[2], number.to_string());
        assert_eq!(call.args[3], comment);

        call.ret
    }

    fn create_branch(&self, owner: &str, repo: &str, branch_name: &str, sha: &str) -> Result<()> {
        let mut calls = self.create_branch_calls.lock().unwrap();
        assert!(calls.len() > 0, "Unexpected call to create_branch");
        let call = calls.remove(0);
        assert_eq!(call.args[0], owner);
        assert_eq!(call.args[1], repo);
        assert_eq!(call.args[2], branch_name);
        assert_eq!(call.args[3], sha);

        call.ret
    }

    fn delete_branch(&self, owner: &str, repo: &str, branch_name: &str) -> Result<()> {
        let mut calls = self.create_branch_calls.lock().unwrap();
        assert!(calls.len() > 0, "Unexpected call to delete_branch");
        let call = calls.remove(0);
        assert_eq!(call.args[0], owner);
        assert_eq!(call.args[1], repo);
        assert_eq!(call.args[2], branch_name);

        call.ret
    }

    fn get_statuses(&self, owner: &str, repo: &str, ref_name: &str) -> Result<Vec<Status>> {
        let mut calls = self.get_status_calls.lock().unwrap();
        assert!(calls.len() > 0, "Unexpected call to get_statuses");
        let call = calls.remove(0);
        assert_eq!(call.args[0], owner);
        assert_eq!(call.args[1], repo);
        assert_eq!(call.args[2], ref_name);

        call.ret
    }

    fn create_status(&self, owner: &str, repo: &str, ref_name: &str, status: &Status) -> Result<()> {
        let mut calls = self.create_status_calls.lock().unwrap();
        assert!(calls.len() > 0, "Unexpected call to create_status");
        let call = calls.remove(0);
        assert_eq!(call.args[0], owner);
        assert_eq!(call.args[1], repo);
        assert_eq!(call.args[2], ref_name);
        assert_eq!(call.args[3], format!("{:?}", status));

        call.ret
    }
}

impl MockGithub {
    pub fn get_pull_request(&self, owner: &str, repo: &str, number: u32, ret: Result<PullRequest>) {
        self.get_pr_calls.lock().unwrap().push(MockCall::new(
            ret,
            vec![owner, repo, &number.to_string()],
        ));
    }

    pub fn mock_get_pull_requests(
        &self,
        owner: &str,
        repo: &str,
        state: Option<&str>,
        head: Option<&str>,
        ret: Result<Vec<PullRequest>>,
    ) {
        self.get_prs_calls.lock().unwrap().push(MockCall::new(
            ret,
            vec![
                owner,
                repo,
                state.unwrap_or(""),
                head.unwrap_or(""),
            ],
        ));
    }

    pub fn mock_create_pull_request(
        &self,
        owner: &str,
        repo: &str,
        title: &str,
        body: &str,
        head: &str,
        base: &str,
        ret: Result<PullRequest>,
    ) {
        self.create_pr_calls.lock().unwrap().push(MockCall::new(
            ret,
            vec![owner, repo, title, body, head, base],
        ));
    }

    pub fn mock_get_pull_request_labels(&self, owner: &str, repo: &str, number: u32, ret: Result<Vec<Label>>) {
        self.get_pr_labels_calls.lock().unwrap().push(MockCall::new(
            ret,
            vec![owner, repo, &number.to_string()],
        ));
    }

    pub fn mock_get_pull_request_commits(&self, owner: &str, repo: &str, number: u32, ret: Result<Vec<Commit>>) {
        self.get_pr_commits_calls.lock().unwrap().push(MockCall::new(
            ret,
            vec![owner, repo, &number.to_string()],
        ));
    }

    pub fn mock_get_pull_request_reviews(&self, owner: &str, repo: &str, number: u32, ret: Result<Vec<Review>>) {
        self.get_pr_reviews_calls.lock().unwrap().push(MockCall::new(
            ret,
            vec![owner, repo, &number.to_string()],
        ));
    }

    pub fn mock_comment_pull_request(&self, owner: &str, repo: &str, number: u32, comment: &str, ret: Result<()>) {
        self.comment_pr_calls.lock().unwrap().push(MockCall::new(
            ret,
            vec![owner, repo, &number.to_string(), comment],
        ));
    }

    pub fn mock_assign_pull_request(
        &self,
        owner: &str,
        repo: &str,
        number: u32,
        assignees: Vec<String>,
        ret: Result<AssignResponse>,
    ) {
        self.assign_pr_calls.lock().unwrap().push(MockCall::new(
            ret,
            vec![
                owner,
                repo,
                &number.to_string(),
                &assignees.join(","),
            ],
        ));
    }

    pub fn mock_create_branch(&self, owner: &str, repo: &str, branch_name: &str, sha: &str, ret: Result<()>) {
        self.create_branch_calls.lock().unwrap().push(MockCall::new(
            ret,
            vec![owner, repo, branch_name, sha],
        ));
    }

    pub fn mock_delete_branch(&self, owner: &str, repo: &str, branch_name: &str, ret: Result<()>) {
        self.delete_branch_calls.lock().unwrap().push(
            MockCall::new(ret, vec![owner, repo, branch_name]),
        );
    }

    pub fn mock_get_statuses(&self, owner: &str, repo: &str, ref_name: &str, ret: Result<Vec<Status>>) {
        self.get_status_calls.lock().unwrap().push(
            MockCall::new(ret, vec![owner, repo, ref_name]),
        );
    }

    pub fn mock_create_status(&self, owner: &str, repo: &str, ref_name: &str, status: &Status, ret: Result<()>) {
        self.create_status_calls.lock().unwrap().push(MockCall::new(
            ret,
            vec![
                owner,
                repo,
                ref_name,
                &format!("{:?}", status),
            ],
        ));
    }
}
