extern crate octobot;
#[macro_use]
extern crate maplit;

mod mocks;

use octobot::config::JiraConfig;
use octobot::jira;
use octobot::jira::*;
use octobot::github;
use octobot::version;

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
        fix_versions_field: Some("the-versions".into()),
        pending_versions_field: Some("the-pending-versions".into()),
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

fn new_push_commit(msg: &str, hash: &str) -> github::PushCommit {
    let mut commit = github::PushCommit::new();
    commit.message = msg.into();
    commit.id = hash.into();
    commit.url = format!("http://the-commit/{}", hash);
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
    let projects = vec!["SER".to_string(), "CLI".to_string()];
    let commit = new_commit("Fix [SER-1] I fixed it. And also relates to [CLI-9999][OTHER-999]", "aabbccddee");

    test.jira.mock_comment_issue("SER-1", "Review submitted for branch master: http://the-pr", Ok(()));

    test.jira.mock_get_transitions("SER-1", Ok(vec![new_transition("001", "progress1")]));
    test.jira.mock_transition_issue("SER-1", &new_transition_req("001"), Ok(()));

    test.jira.mock_get_transitions("SER-1", Ok(vec![new_transition("002", "reviewing1")]));
    test.jira.mock_transition_issue("SER-1", &new_transition_req("002"), Ok(()));

    test.jira.mock_comment_issue("CLI-9999", "Referenced by review submitted for branch master: http://the-pr", Ok(()));

    // mentioned JIRAs should go to in-progress but not "pending review"
    test.jira.mock_get_transitions("CLI-9999", Ok(vec![new_transition("001", "progress1")]));
    test.jira.mock_transition_issue("CLI-9999", &new_transition_req("001"), Ok(()));

    jira::workflow::submit_for_review(&pr, &vec![commit], &projects, &test.jira, &test.config);
}

#[test]
fn test_resolve_issue_no_resolution() {
    let test = new_test();
    let projects = vec!["SER".to_string(), "CLI".to_string()];
    let commit1 = new_push_commit("Fix [SER-1] I fixed it. And also fix [CLI-9999][OTHER-999]\n\n\n\n", "aabbccddee");
    let commit2 = new_push_commit("Really fix [CLI-9999]\n\n\n\n", "ffbbccddee");

    let comment1 = "Merged into branch master: [aabbccd|http://the-commit/aabbccddee]\n\
                   {quote}Fix [SER-1] I fixed it. And also fix [CLI-9999][OTHER-999]{quote}";
    let comment2 = "Merged into branch master: [ffbbccd|http://the-commit/ffbbccddee]\n\
                    {quote}Really fix [CLI-9999]{quote}";

    test.jira.mock_comment_issue("CLI-9999", comment1, Ok(()));
    test.jira.mock_comment_issue("SER-1", comment1, Ok(()));
    test.jira.mock_comment_issue("CLI-9999", comment2, Ok(()));

    // commit 1
    test.jira.mock_get_transitions("CLI-9999", Ok(vec![new_transition("004", "resolved2")]));
    test.jira.mock_transition_issue("CLI-9999", &new_transition_req("004"), Ok(()));
    test.jira.mock_get_transitions("SER-1", Ok(vec![new_transition("003", "resolved1")]));
    test.jira.mock_transition_issue("SER-1", &new_transition_req("003"), Ok(()));

    // commit 2
    test.jira.mock_get_transitions("CLI-9999", Ok(vec![new_transition("004", "resolved2")]));
    test.jira.mock_transition_issue("CLI-9999", &new_transition_req("004"), Ok(()));

    jira::workflow::resolve_issue("master", None, &vec![commit1, commit2], &projects, &test.jira, &test.config);
}

#[test]
fn test_resolve_issue_with_resolution() {
    let test = new_test();
    let projects = vec!["SER".to_string(), "CLI".to_string()];
    let commit = new_push_commit("Fix [SER-1] I fixed it.\n\nand it is kinda related to [CLI-45][OTHER-999]", "aabbccddee");

    let comment1 = "Merged into branch release/99: [aabbccd|http://the-commit/aabbccddee]\n\
                   {quote}Fix [SER-1] I fixed it.{quote}\n\
                   Included in version 5.6.7";
    test.jira.mock_comment_issue("SER-1", comment1, Ok(()));

    let comment2 = "Referenced by commit merged into branch release/99: [aabbccd|http://the-commit/aabbccddee]\n\
                   {quote}Fix [SER-1] I fixed it.{quote}\n\
                   Included in version 5.6.7";
    test.jira.mock_comment_issue("CLI-45", comment2, Ok(()));


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

    jira::workflow::resolve_issue("release/99", Some("5.6.7"), &vec![commit], &projects, &test.jira, &test.config);
}

#[test]
fn test_add_pending_version() {
    let test = new_test();
    let projects = vec!["SER".to_string(), "CLI".to_string()];
    let commit = new_push_commit("Fix [SER-1] I fixed it.\n\nand it is kinda related to [CLI-45][OTHER-999]", "aabbccddee");

    test.jira.mock_add_pending_version("CLI-45", "5.6.7", Ok(()));
    test.jira.mock_add_pending_version("SER-1", "5.6.7", Ok(()));

    jira::workflow::add_pending_version(Some("5.6.7"), &vec![commit], &projects, &test.jira);
}

#[test]
fn test_merge_pending_versions_for_real() {
    let test = new_test();

    let new_version = "1.2.0.500";
    test.jira.mock_get_versions("SER", Ok(vec![Version::new("1.2.0.100"), Version::new("1.3.0.100")]));
    test.jira.mock_find_pending_versions("SER", Ok(hashmap!{
        "SER-1".to_string() => vec![
            version::Version::parse("1.2.0.50").unwrap(),
            version::Version::parse("1.2.0.200").unwrap()
        ],
        "SER-2".to_string() => vec![],
        "SER-3".to_string() => vec![
            version::Version::parse("9.9.9.9").unwrap()
        ],
        "SER-4".to_string() => vec![
            version::Version::parse("1.2.0.700").unwrap(),
            version::Version::parse("1.2.0.300").unwrap()
        ],
    }));

    test.jira.mock_add_version("SER", new_version, Ok(()));

    test.jira.mock_remove_pending_versions("SER-1", &vec![version::Version::parse("1.2.0.200").unwrap()], Ok(()));
    test.jira.mock_remove_pending_versions("SER-4", &vec![version::Version::parse("1.2.0.300").unwrap()], Ok(()));

    test.jira.mock_assign_fix_version("SER-1", new_version, Ok(()));
    test.jira.mock_assign_fix_version("SER-4", new_version, Ok(()));

    let res = hashmap! {
        "SER-1".to_string() => vec![version::Version::parse("1.2.0.200").unwrap()],
        "SER-4".to_string() => vec![version::Version::parse("1.2.0.300").unwrap()],
    };

    assert_eq!(Ok(res), jira::workflow::merge_pending_versions(new_version, "SER", &test.jira, jira::workflow::DryRunMode::ForReal));
}

#[test]
fn test_merge_pending_versions_dry_run() {
    let test = new_test();

    let new_version = "1.2.0.500";
    test.jira.mock_get_versions("SER", Ok(vec![Version::new("1.2.0.100"), Version::new("1.3.0.100")]));
    test.jira.mock_find_pending_versions("SER", Ok(hashmap!{
        "SER-1".to_string() => vec![
            version::Version::parse("1.2.0.50").unwrap(),
            version::Version::parse("1.2.0.200").unwrap()
        ],
        "SER-2".to_string() => vec![],
        "SER-3".to_string() => vec![
            version::Version::parse("9.9.9.9").unwrap()
        ],
        "SER-4".to_string() => vec![
            version::Version::parse("1.2.0.700").unwrap(),
            version::Version::parse("1.2.0.300").unwrap()
        ],
    }));

    // Don't expect the other state-changing functions to get called

    let res = hashmap! {
        "SER-1".to_string() => vec![version::Version::parse("1.2.0.200").unwrap()],
        "SER-4".to_string() => vec![version::Version::parse("1.2.0.300").unwrap()],
    };

    assert_eq!(Ok(res), jira::workflow::merge_pending_versions(new_version, "SER", &test.jira, jira::workflow::DryRunMode::DryRun));
}


#[test]
fn test_merge_pending_versions_missed_versions() {
    let test = new_test();

    let missed_version = "1.2.0.500";
    test.jira.mock_get_versions("SER", Ok(vec![Version::new("1.2.0.100"), Version::new("1.2.0.500"), Version::new("1.2.0.600")]));
    test.jira.mock_find_pending_versions("SER", Ok(hashmap!{
        "SER-1".to_string() => vec![
            version::Version::parse("1.2.0.50").unwrap(),
            version::Version::parse("1.2.0.150").unwrap(),
            version::Version::parse("1.2.0.600").unwrap(),
        ],
    }));

    // Note: don't mock `add_version` since the version already exists

    test.jira.mock_remove_pending_versions("SER-1", &vec![version::Version::parse("1.2.0.150").unwrap()], Ok(()));
    test.jira.mock_assign_fix_version("SER-1", missed_version, Ok(()));

    let res = hashmap! {
        "SER-1".to_string() => vec![version::Version::parse("1.2.0.150").unwrap()],
    };

    assert_eq!(Ok(res), jira::workflow::merge_pending_versions(missed_version, "SER", &test.jira, jira::workflow::DryRunMode::ForReal));
}
