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

fn get_release_note<T: CommitLike>(commit: &T) -> Option<String> {
    // Release-Note [multi-line release note content] Release-Note
    let re = Regex::new(r"(?ims)Release-Note\s*(.*?)\s*Release-Note").unwrap();

    re.captures(commit.message())                   // Find regex matches in commit message
        .and_then(|c| c.get(1))                     // Extract capture group 1 (the content between Release-Note tags)
        .map(|m| m.as_str().trim().to_string())     // Convert to string and trim whitespace from start/end
        .filter(|s| !s.is_empty())                  // Return None if the trimmed content is empty
        .map(|s| {                                  // Enforce 1000 character limit
            if s.len() > 1000 {
                format!("{}... [truncated]", &s[..997])
            } else {
                s
            }
        })
}

fn get_fixed_jira_keys<T: CommitLike>(commits: &[T], projects: &[String]) -> Vec<String> {
    // Fix [ABC-123][OTHER-567], [YEAH-999]
    let re =
        Regex::new(r"(?i)(?:Fix(?:es|ed)?):?\s*(?-i)((\[?([A-Z0-9]+-[0-9]+)(?:\]|\b)[\s,]*)+)")
            .unwrap();

    // first extract jiras with fix markers
    let mut all_refs = vec![];
    for c in commits {
        all_refs.extend(re.captures_iter(c.message()).map(|c| c[1].to_string()));
    }

    get_jira_keys(all_refs, projects)
}

fn get_mentioned_jira_keys<T: CommitLike>(commits: &[T], projects: &[String]) -> Vec<String> {
    // See [ABC-123][OTHER-567], [YEAH-999]
    let re = Regex::new(r"(?i)(?:See):?\s*(?-i)((\[?([A-Z0-9]+-[0-9]+)(?:\]|\b)[\s,]*)+)").unwrap();

    // first extract jiras with see markers
    let mut all_refs = vec![];
    for c in commits {
        all_refs.extend(re.captures_iter(c.message()).map(|c| c[1].to_string()));
    }

    get_jira_keys(all_refs, projects)
}

fn get_referenced_jira_keys<T: CommitLike>(commits: &[T], projects: &[String]) -> Vec<String> {
    let fixed = get_fixed_jira_keys(commits, projects);

    let mut refd = get_all_jira_keys(commits, projects);

    // only return ones not marked as fixed
    refd.retain(|s| !fixed.iter().any(|s2| s == s2));
    refd
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
    if let Some(ref state) = state {
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
    let review_states = config.review_states();
    let progress_states = config.progress_states();

    for key in get_fixed_jira_keys(commits, projects) {
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

        if !needs_transition(&issue_state, &review_states) {
            continue;
        }

        // try to transition to in-progress
        if needs_transition(&issue_state, &progress_states) {
            try_transition(&key, &progress_states, jira).await;
        }

        // try transition to pending-review
        try_transition(&key, &review_states, jira).await;
    }

    let mentioned = get_mentioned_jira_keys(commits, projects);
    for key in get_referenced_jira_keys(commits, projects) {
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

        if mentioned.contains(&key) {
            continue; // don't transition
        }

        let issue_state = try_get_issue_state(&key, jira).await;

        if !needs_transition(&issue_state, &progress_states) {
            continue;
        }

        // try to transition to in-progress
        try_transition(&key, &progress_states, jira).await;
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

        let release_note_desc = match get_release_note(commit) {
            None => String::new(),
            Some(note) => format!("\nRelease-Note\n{}\nRelease Note", note),
        };

        let fix_msg = format!("Merged into branch {}: {}{}{}", branch, desc, version_desc, release_note_desc);
        let ref_msg = format!(
            "Referenced by commit merged into branch {}: {}{}",
            branch, desc, version_desc
        );
        let resolved_states = config.resolved_states();

        for key in get_fixed_jira_keys(&[commit], projects) {
            if let Err(e) = jira.comment_issue(&key, &fix_msg).await {
                error!("Error commenting on key [{}]: {}", key, e);
            }

            // Update release note text field if configured and release note exists
            if let Some(release_note) = get_release_note(commit) {
                if let Err(e) = jira.set_release_note_text(&key, &release_note).await {
                    error!("Error setting release note text for key [{}]: {}", key, e);
                } else {
                    info!("Updated release note text for [{}]", key);
                    
                    // Update release note status to "Complete" when release note text is successfully set
                    if let Err(e) = jira.set_release_note_status(&key, "Complete").await {
                        error!("Error setting release note status for key [{}]: {}", key, e);
                    } else {
                        info!("Updated release note status to 'Complete' for [{}]", key);
                    }
                }
            }

            let issue_state = try_get_issue_state(&key, jira).await;
            if !needs_transition(&issue_state, &resolved_states) {
                continue;
            }

            match find_transition(&key, &resolved_states, jira).await {
                Ok(Some(transition)) => {
                    let mut req = transition.new_request();

                    if let Some(ref fields) = transition.fields {
                        if let Some(ref resolution) = fields.resolution {
                            for res in &resolution.allowed_values {
                                for resolution in config.fixed_resolutions() {
                                    if res.name == resolution {
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

        // add comment only to referenced jiras
        for key in get_referenced_jira_keys(&[commit], projects) {
            if let Err(e) = jira.comment_issue(&key, &ref_msg).await {
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
        let mentioned = get_mentioned_jira_keys(commits, projects);
        for key in get_all_jira_keys(commits, projects) {
            if mentioned.contains(&key) {
                // don't add a pending version
                continue;
            }
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
    let latest_prior_real_version = parse_jira_versions(real_versions)
        .iter()
        .filter(|v| {
            v.major() == target_version.major()
                && v.minor() == target_version.minor()
                && v < &target_version
        })
        .max()
        .cloned()
        .unwrap_or_else(|| version::Version::parse("0.0.0.0").unwrap());

    pending_versions
        .iter()
        .filter_map(|version| {
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
        Ok(issue) => issue.status,
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
        assert_eq!(
            Vec::<String>::new(),
            get_fixed_jira_keys(&[commit.clone()], &projects)
        );
        assert_eq!(
            Vec::<String>::new(),
            get_referenced_jira_keys(&[commit.clone()], &projects)
        );

        commit.commit.message = "Fix [KEY-1][KEY-2], [KEY-3] Some thing that also fixed [KEY-4] which somehow fixes KEY-5"
            .into();
        assert_eq!(
            vec!["KEY-1", "KEY-2", "KEY-3", "KEY-4", "KEY-5"],
            get_fixed_jira_keys(&[commit.clone()], &projects)
        );

        commit.commit.message +=
            "\n\nFix: [KEY-6], and also mentions [KEY-6], [KEY-7] but not [lowercase-99]";
        assert_eq!(
            vec!["KEY-1", "KEY-2", "KEY-3", "KEY-4", "KEY-5", "KEY-6"],
            get_fixed_jira_keys(&[commit.clone()], &projects)
        );
        assert_eq!(
            vec!["KEY-7"],
            get_referenced_jira_keys(&[commit.clone()], &projects)
        );
    }

    #[test]
    pub fn test_get_jira_keys_alt_format() {
        let projects = vec!["KEY".to_string(), "OTHER".to_string()];
        let mut commit = Commit::new();
        commit.commit.message =
            "KEY-1, KEY-2:Some thing that also fixed\n\nAlso [KEY-3], OTHER-5".into();
        assert_eq!(
            Vec::<String>::new(),
            get_fixed_jira_keys(&[commit.clone()], &projects)
        );
        assert_eq!(
            vec!["KEY-1", "KEY-2", "KEY-3", "OTHER-5"],
            get_referenced_jira_keys(&[commit], &projects)
        );
    }

    #[test]
    pub fn test_get_jira_keys_not_allowed_project() {
        let projects = vec!["KEY".to_string()];
        let mut commit = Commit::new();
        commit.commit.message = "KEY-1, OTHER-2:Fixed stuff".into();
        assert_eq!(
            vec!["KEY-1"],
            get_referenced_jira_keys(&[commit], &projects)
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
    fn test_get_release_note() {
        let mut commit = Commit::new();

        // Test commit with no release note
        commit.commit.message = "Fix [KEY-1] Some bug fix".into();
        assert_eq!(None, get_release_note(&commit));

        // Test example 1: content on same line and multiple lines
        commit.commit.message = "Fix [KEY-1]\n\nRelease-Note abc\nxyz\ns kkk\nddd   Release-Note".into();
        assert_eq!(Some("abc\nxyz\ns kkk\nddd".to_string()), get_release_note(&commit));

        // Test example 2: single line content
        commit.commit.message = "Fix [KEY-2]\n\nRelease-Note\nxyz Release-Note".into();
        assert_eq!(Some("xyz".to_string()), get_release_note(&commit));

        // Test traditional multi-line release note
        commit.commit.message = "Fix [KEY-3] Another fix\n\nRelease-Note\nAdded new feature for users.\nImproved performance significantly.\nRelease-Note".into();
        assert_eq!(Some("Added new feature for users.\nImproved performance significantly.".to_string()), get_release_note(&commit));

        // Test case insensitive matching
        commit.commit.message = "Fix [KEY-4] Yet another fix\n\nrelease note\nCase insensitive test\nRELEASE NOTE".into();
        assert_eq!(Some("Case insensitive test".to_string()), get_release_note(&commit));

        // Test with extra spaces and whitespace
        commit.commit.message = "Fix [KEY-5] Fix with spaces\n\nRelease   Note  \n  Extra spaces handled  \n  Release-Note".into();
        assert_eq!(Some("Extra spaces handled".to_string()), get_release_note(&commit));

        // Test empty release note (should return None)
        commit.commit.message = "Fix [KEY-6] Empty note\n\nRelease-Note\n\nRelease-Note".into();
        assert_eq!(None, get_release_note(&commit));

        // Test inline release note
        commit.commit.message = "Fix [KEY-7] Inline\n\nRelease-Note Fixed inline issue Release-Note".into();
        assert_eq!(Some("Fixed inline issue".to_string()), get_release_note(&commit));

        // Test release note with special characters
        commit.commit.message = "Fix [KEY-8] Special chars\n\nRelease-Note\nFixed issue with special chars: & < > % $\nRelease-Note".into();
        assert_eq!(Some("Fixed issue with special chars: & < > % $".to_string()), get_release_note(&commit));

        // Test release note under character limit (should be unchanged)
        let short_note = "A".repeat(999);
        commit.commit.message = format!("Fix [KEY-9] Short note\n\nRelease-Note\n{}\nRelease-Note", short_note);
        assert_eq!(Some(short_note), get_release_note(&commit));

        // Test release note at exact character limit (should be unchanged)
        let exact_limit_note = "A".repeat(1000);
        commit.commit.message = format!("Fix [KEY-10] Exact limit\n\nRelease-Note\n{}\nRelease-Note", exact_limit_note);
        assert_eq!(Some(exact_limit_note), get_release_note(&commit));

        // Test release note just over character limit (should be truncated)
        let just_over_note = "A".repeat(1001);
        commit.commit.message = format!("Fix [KEY-11] Just over\n\nRelease-Note\n{}\nRelease-Note", just_over_note);
        let expected_just_over = format!("{}... [truncated]", "A".repeat(997));
        assert_eq!(Some(expected_just_over), get_release_note(&commit));

        // Test release note well over character limit (should be truncated)
        let long_note = "A".repeat(1500);
        commit.commit.message = format!("Fix [KEY-12] Long note\n\nRelease-Note\n{}\nRelease-Note", long_note);
        let expected_truncated = format!("{}... [truncated]", "A".repeat(997));
        assert_eq!(Some(expected_truncated), get_release_note(&commit));
    }
}
