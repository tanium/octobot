extern crate octobot;

mod mocks;

use octobot::config::JiraConfig;
use octobot::jira;
use octobot::jira::*;
use octobot::github;

use mocks::mock_jira::MockJira;

struct JiraWorkflowTest {
    jira: MockJira,
    config: JiraConfig,
}

fn new_test() -> JiraWorkflowTest {
    let jira = MockJira::new();
    let config = JiraConfig {
        host: "the-host".into(),
        username: "the-jira-user".into(),
        password: "the-jira-pass".into(),
        progress_states: Some(vec!["progress1".into()]),
        review_states: Some(vec!["reviewing1".into()]),
        resolved_states: Some(vec!["resolved1".into(), "resolved2".into()]),
        fixed_resolutions: Some(vec!["it-is-fixed".into()]),
    };

    JiraWorkflowTest {
        jira: jira,
        config: config,
    }
}

fn new_pr() -> github::PullRequest {
    let mut pr = github::PullRequest::new();
    pr.head.ref_name = "pr-branch".into();
    pr.base.ref_name = "master".into();
    pr.html_url = "http://the-pr".into();
    pr
}

fn new_commit(msg: &str, hash: &str) -> github::Commit {
    let mut commit = github::Commit::new();
    commit.commit.message = msg.into();
    commit.sha = hash.into();
    commit.html_url = format!("http://the-commit/{}", hash);
    commit
}

fn new_transition(id: &str, name: &str) -> Transition {
    Transition {
        id: id.into(),
        name: name.into(),
        to: TransitionTo {
            id: String::new(),
            name: format!("{}-inner", name),
        },
        fields: None,
    }
}

fn new_transition_req(id: &str) -> TransitionRequest {
    TransitionRequest {
        transition: IDOrName {
            id: Some(id.into()),
            name: None,
        },
        fields: None,
    }
}

#[test]
fn test_submit_for_review() {
    let test = new_test();
    let pr = new_pr();
    let commit = new_commit("[SER-1] I fixed it. And also [CLI-9999]", "aabbccddee");

    test.jira.mock_comment_issue("CLI-9999", "Review submitted for branch master: http://the-pr", Ok(()));
    test.jira.mock_comment_issue("SER-1", "Review submitted for branch master: http://the-pr", Ok(()));

    // empty twice: once for in-progress, once for in-review
    test.jira.mock_get_transitions("CLI-9999", Ok(vec![new_transition("009", "other")]));
    test.jira.mock_get_transitions("CLI-9999", Ok(vec![new_transition("009", "other")]));

    test.jira.mock_get_transitions("SER-1", Ok(vec![new_transition("001", "progress1")]));
    test.jira.mock_transition_issue("SER-1", &new_transition_req("001"), Ok(()));

    test.jira.mock_get_transitions("SER-1", Ok(vec![new_transition("002", "reviewing1")]));
    test.jira.mock_transition_issue("SER-1", &new_transition_req("002"), Ok(()));

    jira::workflow::submit_for_review(&pr, &vec![commit], &test.jira, &test.config);
}

#[test]
fn test_resolve_issue_no_resolution() {
    let test = new_test();
    let pr = new_pr();
    let commit1 = new_commit("[SER-1] I fixed it. And also [CLI-9999]\n\n\n\n", "aabbccddee");
    let commit2 = new_commit("Really fix [CLI-9999]\n\n\n\n", "ffbbccddee");

    let comment = "Merged into branch master: http://the-pr\n\n\
                   [aabbccd|http://the-commit/aabbccddee]\n{quote}[SER-1] I fixed it. And also [CLI-9999]\n\n\n\n{quote}\n\
                   [ffbbccd|http://the-commit/ffbbccddee]\n{quote}Really fix [CLI-9999]\n\n\n\n{quote}";
    test.jira.mock_comment_issue("CLI-9999", comment, Ok(()));
    test.jira.mock_comment_issue("SER-1", comment, Ok(()));

    test.jira.mock_get_transitions("CLI-9999", Ok(vec![new_transition("004", "resolved2")]));
    test.jira.mock_transition_issue("CLI-9999", &new_transition_req("004"), Ok(()));

    test.jira.mock_get_transitions("SER-1", Ok(vec![new_transition("003", "resolved1")]));
    test.jira.mock_transition_issue("SER-1", &new_transition_req("003"), Ok(()));

    jira::workflow::resolve_issue(&pr, &vec![commit1, commit2], &test.jira, &test.config);
}

#[test]
fn test_resolve_issue_with_resolution() {
    let test = new_test();
    let pr = new_pr();
    let commit = new_commit("[SER-1] I fixed it.\n\nand now I'm saying something about it", "aabbccddee");

    let comment = "Merged into branch master: http://the-pr\n\n\
                  [aabbccd|http://the-commit/aabbccddee]\n{quote}[SER-1] I fixed it.\n\nand now I'm saying something about it{quote}";
    test.jira.mock_comment_issue("SER-1", comment, Ok(()));

    let mut trans = new_transition("003", "resolved1");
    trans.fields = Some(TransitionFields {
        resolution: Some(TransitionField {
            allowed_values: vec![
                Resolution {
                    id: "010".into(),
                    name: "wontfix".into(),
                },
                Resolution {
                    id: "020".into(),
                    name: "it-is-fixed".into(),
                },
            ],
        }),
    });

    let mut req = new_transition_req("003");
    req.fields = Some(TransitionFieldsRequest {
        resolution: Some(IDOrName {
            id: None,
            name: Some("it-is-fixed".into()),
        }),
    });

    test.jira.mock_get_transitions("SER-1", Ok(vec![trans]));
    test.jira.mock_transition_issue("SER-1", &req, Ok(()));

    jira::workflow::resolve_issue(&pr, &vec![commit], &test.jira, &test.config);
}

