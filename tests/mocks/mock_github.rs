use std::sync::Mutex;

use octobot::github::*;
use octobot::github::api::Session;

pub struct MockGithub {
    pub user: User,
    pub host: String,
    pub token: String,

    pub get_pull_request_ret: Mutex<Vec<Result<PullRequest, String>>>,
    pub get_pull_requests_ret: Mutex<Vec<Result<Vec<PullRequest>, String>>>,
    pub create_pull_request_ret: Mutex<Vec<Result<PullRequest, String>>>,
    pub get_pull_request_labels_ret: Mutex<Vec<Result<Vec<Label>, String>>>,
    pub assign_pull_request_ret: Mutex<Vec<Result<AssignResponse, String>>>,
    pub comment_pull_request_ret: Mutex<Vec<Result<(), String>>>,
}

impl MockGithub {
    pub fn new(user: User) -> MockGithub {
        MockGithub {
            user: user,
            host: "the-github-host".to_string(),
            token: "the-github-token".to_string(),

            get_pull_request_ret: Mutex::new(vec![]),
            get_pull_requests_ret: Mutex::new(vec![]),
            create_pull_request_ret: Mutex::new(vec![]),
            get_pull_request_labels_ret: Mutex::new(vec![]),
            assign_pull_request_ret: Mutex::new(vec![]),
            comment_pull_request_ret: Mutex::new(vec![]),
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

    fn get_pull_request(&self, owner: &str, repo: &str, number: u32)
                        -> Result<PullRequest, String> {
        self.get_pull_request_ret.lock().unwrap().remove(0)
    }

    fn get_pull_requests(&self, owner: &str, repo: &str, state: Option<&str>, head: Option<&str>)
                         -> Result<Vec<PullRequest>, String> {
        self.get_pull_requests_ret.lock().unwrap().remove(0)
    }

    fn create_pull_request(&self, owner: &str, repo: &str, title: &str, body: &str, head: &str,
                           base: &str)
                           -> Result<PullRequest, String> {
        self.create_pull_request_ret.lock().unwrap().remove(0)
    }

    fn get_pull_request_labels(&self, owner: &str, repo: &str, number: u32)
                               -> Result<Vec<Label>, String> {
        self.get_pull_request_labels_ret.lock().unwrap().remove(0)
    }

    fn assign_pull_request(&self, owner: &str, repo: &str, number: u32, assignees: Vec<String>)
                           -> Result<AssignResponse, String> {
        self.assign_pull_request_ret.lock().unwrap().remove(0)
    }

    fn comment_pull_request(&self, owner: &str, repo: &str, number: u32, comment: &str)
                            -> Result<(), String> {
        self.comment_pull_request_ret.lock().unwrap().remove(0)
    }
}
