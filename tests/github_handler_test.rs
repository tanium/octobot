extern crate iron;
extern crate octobot;

mod mocks;

use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc;

use iron::status;

use octobot::config::Config;
use octobot::repos::RepoConfig;
use octobot::users::UserConfig;
use octobot::github::*;
use octobot::messenger::SlackMessenger;
use octobot::server::github_handler::GithubEventHandler;

use mocks::mock_github::MockGithub;
use mocks::mock_slack::{SlackCall, MockSlack};

fn new_handler() -> GithubEventHandler {
    let github = MockGithub::new(User::new("joe"));
    let (tx, rx) = mpsc::channel();
    let config = Arc::new(Config::new(UserConfig::new(), RepoConfig::new()));
    let slack = MockSlack::new(vec![]);

    GithubEventHandler {
        event: "ping".to_string(),
        data: HookBody::new(),
        action: "".to_string(),
        config: config.clone(),
        messenger: Box::new(SlackMessenger {
            config: config.clone(),
            slack: Rc::new(slack),
        }),
        github_session: Arc::new(github),
        pr_merge: tx.clone(),
    }
}

#[test]
fn test_ping() {
    let mut handler = new_handler();
    handler.event = "ping".to_string();

    let resp = handler.handle_event().unwrap();
    assert_eq!((status::Ok, "ping".into()), resp);
}
