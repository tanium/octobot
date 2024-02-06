mod mocks;

use maplit::hashmap;

use octobot_lib::config::JiraConfig;
use octobot_lib::github;
use octobot_lib::jira;
use octobot_lib::jira::*;
use octobot_lib::version;

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
        restrict_comment_visibility_to_role: None,
        login_suffix: None,
    };

    JiraWorkflowTest { jira, config }
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

fn new_issue(key: &str, status: Option<&str>) -> Issue {
    Issue {
        key: key.into(),
        status: status.map(|s| Status {
            name: s.to_string(),
        }),
    }
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

#[tokio::test]
async fn test_submit_for_review() {
    let test = new_test();
    let pr = new_pr();
    let projects = vec!["SER".to_string(), "CLI".to_string()];
    let commit = new_commit(
        "Fix [SER-1] I fixed it. And also relates to [CLI-9999][OTHER-999]",
        "aabbccddee",
    );

    test.jira.mock_comment_issue(
        "SER-1",
        "Review submitted for branch master: http://the-pr",
        Ok(()),
    );
    test.jira.mock_comment_issue(
        "CLI-9999",
        "Referenced by review submitted for branch master: http://the-pr",
        Ok(()),
    );

    test.jira
        .mock_get_issue("SER-1", Ok(new_issue("SER-1", None)));
    test.jira
        .mock_get_issue("CLI-9999", Ok(new_issue("CLI-9999", None)));

    test.jira
        .mock_get_transitions("SER-1", Ok(vec![new_transition("001", "progress1")]));
    test.jira
        .mock_transition_issue("SER-1", &new_transition_req("001"), Ok(()));

    test.jira
        .mock_get_transitions("SER-1", Ok(vec![new_transition("002", "reviewing1")]));
    test.jira
        .mock_transition_issue("SER-1", &new_transition_req("002"), Ok(()));

    // mentioned JIRAs should go to in-progress but not "pending review"
    test.jira
        .mock_get_transitions("CLI-9999", Ok(vec![new_transition("001", "progress1")]));
    test.jira
        .mock_transition_issue("CLI-9999", &new_transition_req("001"), Ok(()));

    jira::workflow::submit_for_review(&pr, &vec![commit], &projects, &test.jira, &test.config)
        .await;
}

#[tokio::test]
async fn test_resolve_issue_no_resolution() {
    let test = new_test();
    let projects = vec!["SER".to_string(), "CLI".to_string()];
    let commit1 = new_push_commit(
        "Fix [SER-1] I fixed it. And also fix [CLI-9999][OTHER-999]\n\n\n\n",
        "aabbccddee",
    );
    let commit2 = new_push_commit("Really fix [CLI-9999]\n\n\n\n", "ffbbccddee");

    let comment1 = "Merged into branch master: [aabbccd|http://the-commit/aabbccddee]\n\
                   {quote}Fix [SER-1] I fixed it. And also fix [CLI-9999][OTHER-999]{quote}";
    let comment2 = "Merged into branch master: [ffbbccd|http://the-commit/ffbbccddee]\n\
                    {quote}Really fix [CLI-9999]{quote}";

    test.jira.mock_comment_issue("CLI-9999", comment1, Ok(()));
    test.jira.mock_comment_issue("SER-1", comment1, Ok(()));
    test.jira.mock_comment_issue("CLI-9999", comment2, Ok(()));

    // commit 1
    test.jira
        .mock_get_issue("CLI-9999", Ok(new_issue("CLI-9999", None)));
    test.jira
        .mock_get_issue("SER-1", Ok(new_issue("SER-1", None)));
    test.jira
        .mock_get_transitions("CLI-9999", Ok(vec![new_transition("004", "resolved2")]));
    test.jira
        .mock_transition_issue("CLI-9999", &new_transition_req("004"), Ok(()));
    test.jira
        .mock_get_transitions("SER-1", Ok(vec![new_transition("003", "resolved1")]));
    test.jira
        .mock_transition_issue("SER-1", &new_transition_req("003"), Ok(()));

    // commit 2
    // should only transition if necessary
    test.jira
        .mock_get_issue("CLI-9999", Ok(new_issue("CLI-9999", Some("resolved2"))));

    jira::workflow::resolve_issue(
        "master",
        None,
        &vec![commit1, commit2],
        &projects,
        &test.jira,
        &test.config,
    )
    .await;
}

#[tokio::test]
async fn test_resolve_issue_with_resolution() {
    let test = new_test();
    let projects = vec!["SER2".to_string(), "CLI".to_string()];
    let commit = new_push_commit(
        "Fix [SER2-1] I fixed it.\n\nand it is kinda related to [CLI-45][OTHER-999]",
        "aabbccddee",
    );

    let comment1 = "Merged into branch release/99: [aabbccd|http://the-commit/aabbccddee]\n\
                   {quote}Fix [SER2-1] I fixed it.{quote}\n\
                   Included in version 5.6.7";
    test.jira.mock_comment_issue("SER2-1", comment1, Ok(()));

    let comment2 = "Referenced by commit merged into branch release/99: [aabbccd|http://the-commit/aabbccddee]\n\
                   {quote}Fix [SER2-1] I fixed it.{quote}\n\
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

    test.jira
        .mock_get_issue("SER2-1", Ok(new_issue("SER-1", None)));

    test.jira.mock_get_transitions("SER2-1", Ok(vec![trans]));
    test.jira.mock_transition_issue("SER2-1", &req, Ok(()));

    jira::workflow::resolve_issue(
        "release/99",
        Some("5.6.7"),
        &vec![commit],
        &projects,
        &test.jira,
        &test.config,
    )
    .await;
}

#[tokio::test]
async fn test_transition_issues_only_if_necessary() {
    let test = new_test();
    let pr = new_pr();
    let projects = vec!["SER".to_string(), "CLI".to_string()];
    let commit = new_commit(
        "Fix [SER-1][SER-2][SER-3] I fixed it. And also relates to [CLI-9999][CLI-9998][OTHER-999]. See [CLI-1]",
        "aabbccddee",
    );

    test.jira.mock_comment_issue(
        "SER-1",
        "Review submitted for branch master: http://the-pr",
        Ok(()),
    );
    test.jira.mock_comment_issue(
        "SER-2",
        "Review submitted for branch master: http://the-pr",
        Ok(()),
    );
    test.jira.mock_comment_issue(
        "SER-3",
        "Review submitted for branch master: http://the-pr",
        Ok(()),
    );
    // "See:" references should be mentioned but not transitioned
    test.jira.mock_comment_issue(
        "CLI-1",
        "Referenced by review submitted for branch master: http://the-pr",
        Ok(()),
    );
    test.jira.mock_comment_issue(
        "CLI-9998",
        "Referenced by review submitted for branch master: http://the-pr",
        Ok(()),
    );
    test.jira.mock_comment_issue(
        "CLI-9999",
        "Referenced by review submitted for branch master: http://the-pr",
        Ok(()),
    );

    test.jira
        .mock_get_issue("SER-1", Ok(new_issue("SER-1", Some("reviewing1"))));
    test.jira
        .mock_get_issue("SER-2", Ok(new_issue("SER-2", Some("progress1"))));
    test.jira
        .mock_get_issue("SER-3", Ok(new_issue("SER-3", None)));
    test.jira
        .mock_get_issue("CLI-9998", Ok(new_issue("CLI-9998", Some("progress1"))));
    test.jira
        .mock_get_issue("CLI-9999", Ok(new_issue("CLI-9999", None)));

    test.jira
        .mock_get_transitions("SER-2", Ok(vec![new_transition("002", "reviewing1")]));
    test.jira
        .mock_transition_issue("SER-2", &new_transition_req("002"), Ok(()));

    test.jira
        .mock_get_transitions("SER-3", Ok(vec![new_transition("001", "progress1")]));
    test.jira
        .mock_transition_issue("SER-3", &new_transition_req("001"), Ok(()));
    test.jira
        .mock_get_transitions("SER-3", Ok(vec![new_transition("002", "reviewing1")]));
    test.jira
        .mock_transition_issue("SER-3", &new_transition_req("002"), Ok(()));

    // mentioned JIRAs should go to in-progress but not "pending review"
    test.jira
        .mock_get_transitions("CLI-9999", Ok(vec![new_transition("001", "progress1")]));
    test.jira
        .mock_transition_issue("CLI-9999", &new_transition_req("001"), Ok(()));

    jira::workflow::submit_for_review(&pr, &vec![commit], &projects, &test.jira, &test.config)
        .await;
}

#[tokio::test]
async fn test_add_pending_version() {
    let test = new_test();
    let projects = vec!["SER".to_string(), "CLI".to_string()];
    let commit = new_push_commit(
        "Fix [SER-1] I fixed it.\n\nand it is kinda related to [CLI-45][OTHER-999] (See: CLI-123)",
        "aabbccddee",
    );

    test.jira
        .mock_add_pending_version("CLI-45", "5.6.7", Ok(()));
    test.jira.mock_add_pending_version("SER-1", "5.6.7", Ok(()));

    jira::workflow::add_pending_version(Some("5.6.7"), &vec![commit], &projects, &test.jira).await;
}

#[tokio::test]
async fn test_merge_pending_versions_for_real() {
    let test = new_test();

    let new_version = "1.2.0.500";
    test.jira.mock_get_versions(
        "SER",
        Ok(vec![
            Version::with_id("1.2.0.100", "12345"),
            Version::with_id("1.3.0.100", "54321"),
        ]),
    );
    test.jira.mock_find_pending_versions(
        "SER",
        Ok(hashmap! {
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
        }),
    );

    test.jira.mock_add_version(
        "SER",
        new_version,
        Ok(Version::with_id(new_version, "89012")),
    );

    test.jira.mock_remove_pending_versions(
        "SER-1",
        &vec![version::Version::parse("1.2.0.200").unwrap()],
        Ok(()),
    );
    test.jira.mock_remove_pending_versions(
        "SER-4",
        &vec![version::Version::parse("1.2.0.300").unwrap()],
        Ok(()),
    );

    test.jira
        .mock_assign_fix_version("SER-1", new_version, Ok(()));
    test.jira
        .mock_assign_fix_version("SER-4", new_version, Ok(()));

    let res = version::MergedVersion {
        issues: hashmap! {
        "SER-1".to_string() => vec![version::Version::parse("1.2.0.200").unwrap()],
        "SER-4".to_string() => vec![version::Version::parse("1.2.0.300").unwrap()] },
        version_id: Some("89012".to_string()),
    };

    assert_eq!(
        res,
        jira::workflow::merge_pending_versions(
            new_version,
            "SER",
            &test.jira,
            jira::workflow::DryRunMode::ForReal
        )
        .await
        .unwrap()
    );
}

#[tokio::test]
async fn test_merge_pending_versions_dry_run() {
    let test = new_test();

    let new_version = "1.2.0.500";
    test.jira.mock_get_versions(
        "SER",
        Ok(vec![
            Version::with_id("1.2.0.100", "12345"),
            Version::with_id("1.3.0.100", "54321"),
        ]),
    );
    test.jira.mock_find_pending_versions(
        "SER",
        Ok(hashmap! {
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
        }),
    );

    // Don't expect the other state-changing functions to get called

    let res = version::MergedVersion {
        issues: hashmap! {
        "SER-1".to_string() => vec![version::Version::parse("1.2.0.200").unwrap()],
        "SER-4".to_string() => vec![version::Version::parse("1.2.0.300").unwrap()] },
        version_id: None,
    };

    assert_eq!(
        res,
        jira::workflow::merge_pending_versions(
            new_version,
            "SER",
            &test.jira,
            jira::workflow::DryRunMode::DryRun
        )
        .await
        .unwrap()
    );
}

#[tokio::test]
async fn test_merge_pending_versions_missed_versions() {
    let test = new_test();

    let missed_version = "1.2.0.500";
    test.jira.mock_get_versions(
        "SER",
        Ok(vec![
            Version::with_id("1.2.0.100", "12345"),
            Version::with_id("1.2.0.500", "54321"),
            Version::with_id("1.2.0.600", "89012"),
        ]),
    );
    test.jira.mock_find_pending_versions(
        "SER",
        Ok(hashmap! {
            "SER-1".to_string() => vec![
                version::Version::parse("1.2.0.50").unwrap(),
                version::Version::parse("1.2.0.150").unwrap(),
                version::Version::parse("1.2.0.600").unwrap(),
            ],
        }),
    );

    // Note: don't mock `add_version` since the version already exists

    test.jira.mock_remove_pending_versions(
        "SER-1",
        &vec![version::Version::parse("1.2.0.150").unwrap()],
        Ok(()),
    );
    test.jira
        .mock_assign_fix_version("SER-1", missed_version, Ok(()));

    let res = version::MergedVersion {
        issues: hashmap! {
        "SER-1".to_string() => vec![version::Version::parse("1.2.0.150").unwrap()] },
        version_id: Some("54321".to_string()),
    };

    assert_eq!(
        res,
        jira::workflow::merge_pending_versions(
            missed_version,
            "SER",
            &test.jira,
            jira::workflow::DryRunMode::ForReal
        )
        .await
        .unwrap()
    );
}

#[tokio::test]
async fn test_sort_versions() {
    let test = new_test();

    use jira::api::JiraVersionPosition;

    let v0 = Version::new("not really a version");
    let v1 = Version::new("1.2.0.900");
    let v2 = Version::new("1.2.0.500");
    let v3 = Version::new("5.4.0.600");

    test.jira.mock_get_versions(
        "SER",
        Ok(vec![v0.clone(), v1.clone(), v2.clone(), v3.clone()]),
    );

    test.jira
        .mock_reorder_version(&v2, JiraVersionPosition::First, Ok(()));
    test.jira
        .mock_reorder_version(&v1, JiraVersionPosition::After(v2.clone()), Ok(()));
    test.jira
        .mock_reorder_version(&v3, JiraVersionPosition::After(v1.clone()), Ok(()));
    test.jira
        .mock_reorder_version(&v0, JiraVersionPosition::After(v3.clone()), Ok(()));

    assert_eq!(
        (),
        jira::workflow::sort_versions("SER", &test.jira)
            .await
            .unwrap()
    );
}
