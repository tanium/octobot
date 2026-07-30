//! Jira reference handling rules for commit messages:
//!
//! - `Fixes ABC-123` (anywhere): comment + pending version + transition; resolved as fixed
//!   on merge.
//! - `Part of ABC-123` (anywhere): comment + pending version + transition.
//! - `Relates to ABC-123` (only at the start of a line): same as "part of";
//!   a temporary migration measure.
//! - Bare `ABC-123` in the commit title: same as "part of".
//! - Bare `ABC-123` in the commit body: comment only.

use std::cmp::Ordering;
use std::collections::HashMap;

use anyhow::anyhow;
use log::{error, info};
use regex::Regex;

use crate::config::JiraConfig;
use crate::errors::*;
use crate::github::{Commit, CommitLike, PullRequest, PushCommit};
use crate::jira;
use crate::jira::Transition;
use crate::version;

fn get_jira_keys(strings: Vec<String>, projects: &[String]) -> Vec<String> {
    let re = Regex::new(r"\b([A-Z0-9]+-[0-9]+)\b").unwrap();

    let mut all_keys = vec![];
    for s in strings {
        for c in re.captures_iter(&s) {
            let key = c[1].to_string();
            let proj = get_jira_project(&key).to_string();

            if projects.contains(&proj) {
                all_keys.push(key);
            }
        }
    }

    all_keys.sort();
    all_keys.dedup();

    all_keys
}

// Extract jira keys following a marker, e.g. Fix [ABC-123][OTHER-567], [YEAH-999]
fn get_marked_jira_keys<T: CommitLike>(
    commits: &[T],
    marker: &str,
    projects: &[String],
) -> Vec<String> {
    let re = Regex::new(&format!(
        r"{}:?\s*(?-i)((\[?([A-Z0-9]+-[0-9]+)(?:\]|\b)[\s,]*)+)",
        marker
    ))
    .unwrap();

    let mut all_refs = vec![];
    for c in commits {
        all_refs.extend(re.captures_iter(c.message()).map(|c| c[1].to_string()));
    }

    get_jira_keys(all_refs, projects)
}

fn merge_keys(mut a: Vec<String>, b: Vec<String>) -> Vec<String> {
    a.extend(b);
    a.sort();
    a.dedup();
    a
}

fn get_title_jira_keys<T: CommitLike>(commits: &[T], projects: &[String]) -> Vec<String> {
    get_jira_keys(commits.iter().map(|c| Commit::title(c)).collect(), projects)
}

// The three tiers of jira keys found in a set of commits. Every key in the commits lands
// in exactly one tier, and a key in a stronger tier is excluded from the weaker ones:
// fixed > referenced > commented.
struct JiraKeys {
    // Marked with "fix": commented, given a pending version, transitioned to
    // pending-review on submit, and resolved as fixed on merge.
    fixed: Vec<String>,
    // Marked with "part of" (or line-start "relates to"), or bare in a commit title:
    // commented, given a pending version, and transitioned to in-progress on submit.
    referenced: Vec<String>,
    // Bare keys mentioned only in commit bodies: commented, nothing else.
    commented: Vec<String>,
}

fn classify_jira_keys<T: CommitLike>(commits: &[T], projects: &[String]) -> JiraKeys {
    let fixed = get_marked_jira_keys(commits, r"(?i)\bFix(?:es|ed)?", projects);

    let part_of = get_marked_jira_keys(commits, r"(?i)\bPart of", projects);
    // Temporary migration measure: lines starting with "relates to" act like "part of"
    let relates_to = get_marked_jira_keys(commits, r"(?im)^\s*Relates to", projects);
    let titled = get_title_jira_keys(commits, projects);

    let mut referenced = merge_keys(merge_keys(part_of, relates_to), titled);
    referenced.retain(|key| !fixed.contains(key));

    let mut commented = get_all_jira_keys(commits, projects);
    commented.retain(|key| !fixed.contains(key) && !referenced.contains(key));

    JiraKeys {
        fixed,
        referenced,
        commented,
    }
}

pub(crate) fn get_all_jira_keys<T: CommitLike>(commits: &[T], projects: &[String]) -> Vec<String> {
    get_jira_keys(
        commits.iter().map(|c| c.message().to_string()).collect(),
        projects,
    )
}

pub fn references_jira<T: CommitLike>(commits: &[T], project: &str) -> bool {
    let projects = vec![project.to_owned()];

    !get_all_jira_keys(commits, &projects).is_empty()
}

fn get_jira_project(jira_key: &str) -> &str {
    let re = Regex::new(r"^([A-Za-z0-9]+)(-[0-9]+)?$").unwrap();

    match re.captures(jira_key) {
        Some(c) => c.get(1).map_or(jira_key, |m| m.as_str()),
        None => jira_key,
    }
}

fn needs_transition(state: &Option<jira::Status>, target: &[String]) -> bool {
    if let Some(state) = state {
        !target.contains(&state.name)
    } else {
        true
    }
}

pub async fn submit_for_review(
    pr: &PullRequest,
    commits: &[Commit],
    projects: &[String],
    jira: &dyn jira::api::Session,
    config: &JiraConfig,
) {
    let review_states = &config.review_states;
    let progress_states = &config.progress_states;

    let keys = classify_jira_keys(commits, projects);

    for key in keys.fixed {
        // add comment
        if let Err(e) = jira
            .comment_issue(
                &key,
                &format!(
                    "Review submitted for branch {}: {}",
                    pr.base.ref_name, pr.html_url
                ),
            )
            .await
        {
            error!("Error commenting on key [{}]: {}", key, e);
            continue; // give up on transitioning if we can't comment.
        }

        let issue_state = try_get_issue_state(&key, jira).await;

        if issue_state
            .as_ref()
            .is_some_and(|s| config.frozen_states.contains(&s.name))
        {
            // don't transition issues "backwards" from fixed/resolved
            info!("Issue already in state '{issue_state:?}', won't transition",);
            continue;
        }

        if !needs_transition(&issue_state, review_states) {
            continue;
        }

        // try to transition to in-progress
        if needs_transition(&issue_state, progress_states) {
            try_transition(&key, progress_states, jira).await;
        }

        // try transition to pending-review
        try_transition(&key, review_states, jira).await;
    }

    for key in keys.referenced {
        // add comment
        if let Err(e) = jira
            .comment_issue(
                &key,
                &format!(
                    "Referenced by review submitted for branch {}: {}",
                    pr.base.ref_name, pr.html_url
                ),
            )
            .await
        {
            error!("Error commenting on key [{}]: {}", key, e);
            continue; // give up on transitioning if we can't comment.
        }

        let issue_state = try_get_issue_state(&key, jira).await;

        if issue_state
            .as_ref()
            .is_some_and(|s| config.frozen_states.contains(&s.name))
        {
            // don't transition issues "backwards" from fixed/resolved
            info!("Issue already in state '{issue_state:?}', won't transition",);
            continue;
        }

        if !needs_transition(&issue_state, progress_states) {
            continue;
        }

        // try to transition to in-progress
        try_transition(&key, progress_states, jira).await;
    }

    // comment-only keys get a comment, but no transitions
    for key in keys.commented {
        if let Err(e) = jira
            .comment_issue(
                &key,
                &format!(
                    "Mentioned by review submitted for branch {}: {}",
                    pr.base.ref_name, pr.html_url
                ),
            )
            .await
        {
            error!("Error commenting on key [{}]: {}", key, e);
        }
    }
}

pub async fn resolve_issue(
    branch: &str,
    version: Option<&str>,
    commits: &[PushCommit],
    projects: &[String],
    jira: &dyn jira::api::Session,
    config: &JiraConfig,
) {
    // Note: keys are intentionally classified per-commit here, unlike submit_for_review
    // and add_pending_version which classify across the whole set of commits: each commit
    // comments separately on the issues it mentions, and at resolve time only the fixed
    // tier does anything beyond commenting, so a cross-commit tier upgrade could not
    // change any transition.
    for commit in commits {
        let desc = format!(
            "[{}|{}]\n{{quote}}{}{{quote}}",
            Commit::short_hash(&commit),
            commit.html_url(),
            Commit::title(&commit)
        );

        let version_desc = match version {
            None => String::new(),
            Some(v) => format!("\nIncluded in version {}", v),
        };

        let fix_msg = format!("Merged into branch {}: {}{}", branch, desc, version_desc);
        let ref_msg = format!(
            "Referenced by commit merged into branch {}: {}{}",
            branch, desc, version_desc
        );
        let mention_msg = format!(
            "Mentioned by commit merged into branch {}: {}{}",
            branch, desc, version_desc
        );
        let resolved_states = &config.resolved_states;

        let keys = classify_jira_keys(&[commit], projects);

        for key in keys.fixed {
            if let Err(e) = jira.comment_issue(&key, &fix_msg).await {
                error!("Error commenting on key [{}]: {}", key, e);
            }

            let issue_state = try_get_issue_state(&key, jira).await;
            if !needs_transition(&issue_state, resolved_states) {
                continue;
            }

            match find_transition(&key, resolved_states, jira).await {
                Ok(Some(transition)) => {
                    let mut req = transition.new_request();

                    if let Some(ref fields) = transition.fields {
                        if let Some(ref resolution) = fields.resolution {
                            for res in &resolution.allowed_values {
                                for resolution in &config.fixed_resolutions {
                                    if res.name == *resolution {
                                        req.set_resolution(res);
                                        break;
                                    }
                                }
                                if req.fields.is_some() {
                                    break;
                                }
                            }
                            if req.fields.is_none() {
                                error!(
                                    "Could not find fixed resolution in allowed values: [{:?}]!",
                                    resolution.allowed_values
                                );
                            }
                        }
                    }

                    if let Err(e) = jira.transition_issue(&key, &req).await {
                        error!(
                            "Error transitioning JIRA issue [{}] to one of [{:?}]: {}",
                            key, resolved_states, e
                        );
                    } else {
                        info!("Transitioned [{}] to one of [{:?}]", key, resolved_states);
                    }
                }
                Ok(None) => info!(
                    "JIRA [{}] cannot be transitioned to  any of [{:?}]",
                    key, resolved_states
                ),
                Err(e) => error!("{}", e),
            };
        }

        // add comments to the other jiras too
        for key in keys.referenced {
            if let Err(e) = jira.comment_issue(&key, &ref_msg).await {
                error!("Error commenting on key [{}]: {}", key, e);
            }
        }
        for key in keys.commented {
            if let Err(e) = jira.comment_issue(&key, &mention_msg).await {
                error!("Error commenting on key [{}]: {}", key, e);
            }
        }
    }
}

pub async fn add_pending_version(
    maybe_version: Option<&str>,
    commits: &[PushCommit],
    projects: &[String],
    jira: &dyn jira::api::Session,
) {
    if let Some(version) = maybe_version {
        let keys = classify_jira_keys(commits, projects);
        // comment-only keys don't get a pending version
        for key in merge_keys(keys.fixed, keys.referenced) {
            if let Err(e) = jira.add_pending_version(&key, version).await {
                error!(
                    "Error adding pending version {} to key{}: {}",
                    version, key, e
                );
                continue;
            }
        }
    }
}

fn parse_jira_versions(versions: &[jira::Version]) -> Vec<version::Version> {
    versions
        .iter()
        .filter_map(|v| version::Version::parse(&v.name))
        .collect::<Vec<_>>()
}

#[derive(PartialEq)]
pub enum DryRunMode {
    DryRun,
    ForReal,
}

pub async fn merge_pending_versions(
    version: &str,
    project: &str,
    jira: &dyn jira::api::Session,
    mode: DryRunMode,
) -> Result<version::MergedVersion> {
    let target_version = match version::Version::parse(version) {
        Some(v) => v,
        None => return Err(anyhow!("Invalid target version: {}", version)),
    };

    let real_versions = jira.get_versions(project).await?;
    let all_pending_versions = jira.find_pending_versions(project).await?;

    let all_relevant_versions = all_pending_versions
        .iter()
        .filter_map(|(key, list)| {
            let relevant = find_relevant_versions(&target_version, list, &real_versions);
            if relevant.is_empty() {
                None
            } else {
                Some((key.clone(), relevant))
            }
        })
        .collect::<HashMap<_, _>>();

    if mode == DryRunMode::DryRun {
        return Ok(version::MergedVersion {
            issues: all_relevant_versions,
            version_id: None,
        });
    }

    if all_relevant_versions.is_empty() {
        return Err(anyhow!(
            "No relevant pending versions for version {}",
            version
        ));
    }

    // create the target version for this project
    let id = match real_versions.into_iter().find(|v| v.name == version) {
        Some(v) => {
            info!(
                "JIRA version {} already exists for project {}",
                version, project
            );
            v.id
        }
        None => {
            info!(
                "Creating new JIRA version {} for project {}",
                version, project
            );

            jira.add_version(project, version).await?.id
        }
    };

    {
        // sort the keys for deterministic results for testing purposes.
        let mut keys = all_relevant_versions.keys().collect::<Vec<_>>();
        keys.sort();

        // group together relevant versions into this version!
        for key in keys {
            info!("Assigning JIRA version key {}: {}", key, version);
            let relevant_versions = all_relevant_versions.get(key).unwrap();
            if let Err(e) = jira.assign_fix_version(key, version).await {
                error!("Error assigning version {} to key {}: {}", version, key, e);
                continue;
            }

            info!(
                "Removing pending versions key {}: {:?}",
                key, relevant_versions
            );
            if let Err(e) = jira.remove_pending_versions(key, relevant_versions).await {
                error!(
                    "Error clearing pending version {} from key {}: {}",
                    version, key, e
                );
                continue;
            }
        }
    }

    Ok(version::MergedVersion {
        issues: all_relevant_versions,
        version_id: Some(id),
    })
}

fn find_relevant_versions(
    target_version: &version::Version,
    pending_versions: &[version::Version],
    real_versions: &[jira::Version],
) -> Vec<version::Version> {
    // For pre-release targets, real versions (which have no pre-release suffix) will never
    // match, so the cutoff defaults to 0.0.0.0. This is intentional: pre-release channels
    // are continuous and unbounded by shipped releases.
    let latest_prior_real_version = parse_jira_versions(real_versions)
        .iter()
        .filter(|v| {
            v.major() == target_version.major()
                && v.minor() == target_version.minor()
                && v.pre_release() == target_version.pre_release()
                && v < &target_version
        })
        .max()
        .cloned()
        .unwrap_or_else(|| version::Version::parse("0.0.0.0").unwrap());

    pending_versions
        .iter()
        .filter_map(|version| {
            if version.pre_release() != target_version.pre_release() {
                return None;
            }

            if version.major() == target_version.major()
                && version.minor() == target_version.minor()
                && version <= target_version
                && version > &latest_prior_real_version
            {
                Some(version.clone())
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
}

async fn try_get_issue_state(key: &str, jira: &dyn jira::api::Session) -> Option<jira::Status> {
    match jira.get_issue(key).await {
        Ok(issue) => Some(issue.fields.status),
        Err(e) => {
            error!("Error getting JIRA [{}] {}", key, e);
            None
        }
    }
}

async fn try_transition(key: &str, to: &[String], jira: &dyn jira::api::Session) {
    match find_transition(key, to, jira).await {
        Ok(Some(transition)) => {
            let req = transition.new_request();
            if let Err(e) = jira.transition_issue(key, &req).await {
                error!(
                    "Error transitioning JIRA issue [{}] to one of [{:?}]: {}",
                    key, to, e
                );
            } else {
                info!("Transitioned [{}] to one of [{:?}]", key, to);
            }
        }
        Ok(None) => info!("JIRA [{}] cannot be transitioned to any of [{:?}]", key, to),
        Err(e) => error!("{}", e),
    };
}

async fn find_transition(
    key: &str,
    to: &[String],
    jira: &dyn jira::api::Session,
) -> Result<Option<Transition>> {
    let transitions = jira.get_transitions(key).await?;

    Ok(pick_transition(to, &transitions))
}

fn pick_transition(to: &[String], choices: &[Transition]) -> Option<Transition> {
    for t in choices {
        for name in to {
            if &t.name == name || &t.to.name == name {
                return Some(t.clone());
            }
        }
    }

    None
}

pub async fn sort_versions(project: &str, jira: &dyn jira::api::Session) -> Result<()> {
    let mut versions = jira.get_versions(project).await?;

    versions.sort_by(|a, b| {
        let v1 = version::Version::parse(&a.name);
        let v2 = version::Version::parse(&b.name);
        match (v1, v2) {
            (None, None) => a.name.cmp(&b.name),
            (None, Some(_)) => Ordering::Greater,
            (Some(_), None) => Ordering::Less,
            (Some(v1), Some(v2)) => v1.cmp(&v2),
        }
    });

    for i in 0..versions.len() {
        let v = &versions[i];
        if i == 0 {
            jira.reorder_version(v, jira::api::JiraVersionPosition::First)
                .await?;
        } else {
            let prev = &versions[i - 1];
            jira.reorder_version(v, jira::api::JiraVersionPosition::After(prev.clone()))
                .await?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use jira::TransitionTo;

    #[test]
    pub fn test_get_jira_keys() {
        let projects = vec!["KEY".to_string(), "lowercase".to_string()];
        let mut commit = Commit::new();
        let keys = classify_jira_keys(&[commit.clone()], &projects);
        assert_eq!(Vec::<String>::new(), keys.fixed);
        assert_eq!(Vec::<String>::new(), keys.referenced);
        assert_eq!(Vec::<String>::new(), keys.commented);

        commit.commit.message = "Fix [KEY-1][KEY-2], [KEY-3] Some thing that also fixed [KEY-4] which somehow fixes KEY-5"
            .into();
        assert_eq!(
            vec!["KEY-1", "KEY-2", "KEY-3", "KEY-4", "KEY-5"],
            classify_jira_keys(&[commit.clone()], &projects).fixed
        );

        commit.commit.message +=
            "\n\nFix: [KEY-6], and also mentions [KEY-6], [KEY-7] but not [lowercase-99]";
        let keys = classify_jira_keys(&[commit], &projects);
        assert_eq!(
            vec!["KEY-1", "KEY-2", "KEY-3", "KEY-4", "KEY-5", "KEY-6"],
            keys.fixed
        );
        // bare keys in the body get comments only
        assert_eq!(Vec::<String>::new(), keys.referenced);
        assert_eq!(vec!["KEY-7"], keys.commented);
    }

    #[test]
    pub fn test_get_jira_keys_alt_format() {
        let projects = vec!["KEY".to_string(), "OTHER".to_string()];
        let mut commit = Commit::new();
        commit.commit.message =
            "KEY-1, KEY-2:Some thing that also fixed\n\nAlso [KEY-3], OTHER-5".into();
        let keys = classify_jira_keys(&[commit], &projects);
        assert_eq!(Vec::<String>::new(), keys.fixed);
        // bare keys in the title are references, bare keys in the body are comment-only
        assert_eq!(vec!["KEY-1", "KEY-2"], keys.referenced);
        assert_eq!(vec!["KEY-3", "OTHER-5"], keys.commented);
    }

    #[test]
    pub fn test_get_part_of_jira_keys() {
        let projects = vec!["KEY".to_string()];
        let mut commit = Commit::new();
        commit.commit.message =
            "Add the thing\n\nPart of [KEY-1], KEY-2\nAlso part of: KEY-3".into();
        let keys = classify_jira_keys(&[commit], &projects);
        assert_eq!(Vec::<String>::new(), keys.fixed);
        assert_eq!(vec!["KEY-1", "KEY-2", "KEY-3"], keys.referenced);
        assert_eq!(Vec::<String>::new(), keys.commented);
    }

    #[test]
    pub fn test_get_jira_keys_markers_need_word_boundary() {
        let projects = vec!["KEY".to_string()];
        let mut commit = Commit::new();
        commit.commit.message = "Add the thing\n\nhotfix KEY-1 is the counterpart of KEY-2".into();
        let keys = classify_jira_keys(&[commit], &projects);
        // "hotfix" and "counterpart of" are not markers
        assert_eq!(Vec::<String>::new(), keys.fixed);
        assert_eq!(Vec::<String>::new(), keys.referenced);
        assert_eq!(vec!["KEY-1", "KEY-2"], keys.commented);
    }

    #[test]
    pub fn test_get_relates_to_jira_keys_only_at_line_start() {
        let projects = vec!["KEY".to_string()];
        let mut commit = Commit::new();
        commit.commit.message =
            "Add the thing\n\nRelates to [KEY-1], KEY-2\nrelates to: KEY-4\nthis somehow relates to KEY-3"
                .into();
        let keys = classify_jira_keys(&[commit], &projects);
        assert_eq!(vec!["KEY-1", "KEY-2", "KEY-4"], keys.referenced);
        // "relates to" mid-line does not count as a reference
        assert_eq!(vec!["KEY-3"], keys.commented);
    }

    #[test]
    pub fn test_get_jira_keys_see_is_not_special() {
        let projects = vec!["KEY".to_string()];
        let mut commit = Commit::new();
        commit.commit.message = "KEY-1: Add the thing\n\nSee [KEY-2], KEY-3".into();
        let keys = classify_jira_keys(&[commit], &projects);
        assert_eq!(vec!["KEY-1"], keys.referenced);
        assert_eq!(vec!["KEY-2", "KEY-3"], keys.commented);
    }

    #[test]
    pub fn test_get_jira_keys_across_commits() {
        let projects = vec!["KEY".to_string()];
        let mut commit1 = Commit::new();
        commit1.commit.message = "KEY-1: Add the thing".into();
        let mut commit2 = Commit::new();
        commit2.commit.message = "Clean up the thing\n\nMore about KEY-1 and KEY-2".into();

        // a key referenced in any commit title wins over a bare body mention in another
        let keys = classify_jira_keys(&[commit1, commit2], &projects);
        assert_eq!(vec!["KEY-1"], keys.referenced);
        assert_eq!(vec!["KEY-2"], keys.commented);
    }

    #[test]
    pub fn test_get_jira_keys_not_allowed_project() {
        let projects = vec!["KEY".to_string()];
        let mut commit = Commit::new();
        commit.commit.message = "KEY-1, OTHER-2:Fixed stuff".into();
        assert_eq!(
            vec!["KEY-1"],
            classify_jira_keys(&[commit], &projects).referenced
        );
    }

    #[test]
    pub fn test_pick_transition() {
        let t1 = Transition {
            id: "1".into(),
            name: "t1".into(),
            to: TransitionTo {
                id: "10".into(),
                name: "inside-t1".into(),
            },
            fields: None,
        };
        let t2 = Transition {
            id: "2".into(),
            name: "t2".into(),
            to: TransitionTo {
                id: "20".into(),
                name: "inside-t2".into(),
            },
            fields: None,
        };
        assert_eq!(
            Some(t1.clone()),
            pick_transition(&["t1".into()], &[t1.clone(), t2.clone()])
        );
        assert_eq!(
            Some(t1.clone()),
            pick_transition(
                &["inside-t1".into(), "t2".into()],
                &[t1.clone(), t2.clone()]
            )
        );
        assert_eq!(
            Some(t2.clone()),
            pick_transition(&["inside-t2".into()], &[t1.clone(), t2.clone()])
        );
        assert_eq!(None, pick_transition(&["something-else".into()], &[t1, t2]));
    }

    #[test]
    fn test_get_jira_project() {
        assert_eq!("SERVER", get_jira_project("SERVER-123"));
        assert_eq!("BUILD", get_jira_project("BUILD"));
        assert_eq!("doesn't match", get_jira_project("doesn't match"));
    }

    #[test]
    fn test_find_relevant_versions() {
        let target_version = version::Version::parse("3.4.0.1000").unwrap();
        let real_versions = vec![
            // wrong major
            jira::Version::new("2.4.0.000"),
            // wrong minor
            jira::Version::new("3.2.0.000"),
            // we want the max: should ignore
            jira::Version::new("3.4.0.000"),
            jira::Version::new("3.4.0.100"),
            // just right -- should pick this one
            jira::Version::new("3.4.0.400"),
        ];
        let pending_versions = vec![
            // wrong major
            version::Version::parse("2.4.0.500").unwrap(),
            // wrong minor
            version::Version::parse("3.3.0.500").unwrap(),
            // too early
            version::Version::parse("3.4.0.300").unwrap(),
            // too late
            version::Version::parse("3.4.0.1001").unwrap(),
            // just right
            version::Version::parse("3.4.0.500").unwrap(),
            version::Version::parse("3.4.0.600").unwrap(),
        ];
        let expected: Vec<version::Version> = vec![
            version::Version::parse("3.4.0.500").unwrap(),
            version::Version::parse("3.4.0.600").unwrap(),
        ];
        assert_eq!(
            expected,
            find_relevant_versions(&target_version, &pending_versions, &real_versions)
        );
    }

    #[test]
    fn test_find_relevant_versions_inclusive_max() {
        let target_version = version::Version::parse("3.4.0.1000").unwrap();
        let real_versions = vec![jira::Version::new("3.4.0.400")];
        let pending_versions = vec![version::Version::parse("3.4.0.1000").unwrap()];
        let expected: Vec<version::Version> = vec![version::Version::parse("3.4.0.1000").unwrap()];
        assert_eq!(
            expected,
            find_relevant_versions(&target_version, &pending_versions, &real_versions)
        );
    }

    #[test]
    fn test_find_relevant_versions_exclusive_min() {
        let target_version = version::Version::parse("3.4.0.1000").unwrap();
        let real_versions = vec![jira::Version::new("3.4.0.400")];
        let pending_versions = vec![
            version::Version::parse("3.4.0.400").unwrap(),
            version::Version::parse("3.4.0.401").unwrap(),
        ];
        let expected: Vec<version::Version> = vec![version::Version::parse("3.4.0.401").unwrap()];
        assert_eq!(
            expected,
            find_relevant_versions(&target_version, &pending_versions, &real_versions)
        );
    }

    #[test]
    fn test_find_relevant_versions_no_real_versions() {
        let target_version = version::Version::parse("1.2.0.500").unwrap();
        // no real versions --> anything under target matches!
        let real_versions = vec![];
        let pending_versions = vec![
            // major/minor still matter
            version::Version::parse("1.1.0.100").unwrap(),
            version::Version::parse("2.2.0.100").unwrap(),
            // later than target still matters
            version::Version::parse("1.2.0.900").unwrap(),
            // just right
            version::Version::parse("1.2.0.100").unwrap(),
            version::Version::parse("1.2.0.200").unwrap(),
        ];
        let expected: Vec<version::Version> = vec![
            version::Version::parse("1.2.0.100").unwrap(),
            version::Version::parse("1.2.0.200").unwrap(),
        ];
        assert_eq!(
            expected,
            find_relevant_versions(&target_version, &pending_versions, &real_versions)
        );
    }

    #[test]
    fn test_find_relevant_versions_missed_versions() {
        let target_version = version::Version::parse("3.4.0.2000").unwrap();
        let real_versions = vec![
            // our exact target version
            jira::Version::new("3.4.0.2000"),
            // a newer one
            jira::Version::new("3.4.0.3000"),
            // an older one -- should pick this one
            jira::Version::new("3.4.0.1000"),
        ];
        let pending_versions = vec![
            // the one that got missed
            version::Version::parse("3.4.0.1500").unwrap(),
            // too early
            version::Version::parse("3.4.0.1000").unwrap(),
            // too late
            version::Version::parse("3.4.0.2001").unwrap(),
        ];
        let expected: Vec<version::Version> = vec![version::Version::parse("3.4.0.1500").unwrap()];
        assert_eq!(
            expected,
            find_relevant_versions(&target_version, &pending_versions, &real_versions)
        );
    }

    #[test]
    fn test_find_relevant_versions_skips_pre_release() {
        let target_version = version::Version::parse("2026.3.11").unwrap();
        let real_versions = vec![];
        let pending_versions = vec![
            version::Version::parse("2026.3.10").unwrap(),
            version::Version::parse("2026.3.11").unwrap(),
            version::Version::parse("2026.3.11-main").unwrap(),
            version::Version::parse("2026.3.11-staging").unwrap(),
        ];
        let expected: Vec<version::Version> = vec![
            version::Version::parse("2026.3.10").unwrap(),
            version::Version::parse("2026.3.11").unwrap(),
        ];
        assert_eq!(
            expected,
            find_relevant_versions(&target_version, &pending_versions, &real_versions)
        );
    }

    #[test]
    fn test_find_relevant_versions_pre_release_only() {
        let target_version = version::Version::parse("2026.3.11").unwrap();
        let real_versions = vec![];
        let pending_versions = vec![
            version::Version::parse("2026.3.11-main").unwrap(),
            version::Version::parse("2026.3.11-staging").unwrap(),
        ];
        let expected: Vec<version::Version> = vec![];
        assert_eq!(
            expected,
            find_relevant_versions(&target_version, &pending_versions, &real_versions)
        );
    }

    #[test]
    fn test_find_relevant_versions_mixed_with_real() {
        let target_version = version::Version::parse("3.4.0.1000").unwrap();
        let real_versions = vec![jira::Version::new("3.4.0.400")];
        let pending_versions = vec![
            version::Version::parse("3.4.0.500").unwrap(),
            version::Version::parse("3.4.0.500-main").unwrap(),
            version::Version::parse("3.4.0.600").unwrap(),
            version::Version::parse("3.4.0.700-staging").unwrap(),
        ];
        let expected: Vec<version::Version> = vec![
            version::Version::parse("3.4.0.500").unwrap(),
            version::Version::parse("3.4.0.600").unwrap(),
        ];
        assert_eq!(
            expected,
            find_relevant_versions(&target_version, &pending_versions, &real_versions)
        );
    }

    #[test]
    fn test_find_relevant_versions_pre_release_channel_isolation() {
        let target_version = version::Version::parse("2026.3.11-staging").unwrap();
        let real_versions = vec![];
        let pending_versions = vec![
            version::Version::parse("2026.3.10-main").unwrap(),
            version::Version::parse("2026.3.10-staging").unwrap(),
            version::Version::parse("2026.3.11-main").unwrap(),
            version::Version::parse("2026.3.11-staging").unwrap(),
            version::Version::parse("2026.3.11").unwrap(),
        ];
        let expected: Vec<version::Version> = vec![
            version::Version::parse("2026.3.10-staging").unwrap(),
            version::Version::parse("2026.3.11-staging").unwrap(),
        ];
        assert_eq!(
            expected,
            find_relevant_versions(&target_version, &pending_versions, &real_versions)
        );
    }
}
