use regex::Regex;

use config::JiraConfig;
use github::{Commit, CommitLike, PullRequest, PushCommit};
use jira;
use jira::Transition;

fn get_jira_keys(strings: Vec<String>, projects: &Vec<String>) -> Vec<String> {
    let re = Regex::new(r"\b([A-Z]+-[0-9]+)\b").unwrap();

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

fn get_fixed_jira_keys<T: CommitLike>(commits: &Vec<T>, projects: &Vec<String>) -> Vec<String> {
    // Fix [ABC-123][OTHER-567], [YEAH-999]
    let re = Regex::new(r"(?i)(?:Fix(?:es|ed)?):?\s*(?-i)((\[?([A-Z]+-[0-9]+)(?:\]|\b)[\s,]*)+)").unwrap();

    // first extract jiras with fix markers
    let mut all_refs = vec![];
    for c in commits {
        all_refs.extend(re.captures_iter(c.message()).map(|c| c[1].to_string()));
    }

    get_jira_keys(all_refs, projects)
}

fn get_referenced_jira_keys<T: CommitLike>(commits: &Vec<T>, projects: &Vec<String>) -> Vec<String> {
    let fixed = get_fixed_jira_keys(commits, projects);

    let mut refd = get_all_jira_keys(commits, projects);

    // only return ones not marked as fixed
    refd.retain(|s| fixed.iter().position(|s2| s == s2).is_none());
    refd
}

fn get_all_jira_keys<T: CommitLike>(commits: &Vec<T>, projects: &Vec<String>) -> Vec<String> {
    get_jira_keys(commits.iter().map(|c| c.message().to_string()).collect(), projects)
}

fn get_jira_project(jira_key: &str) -> &str {
    let re = Regex::new(r"^([A-Za-z]+)(-[0-9]+)?$").unwrap();

    match re.captures(&jira_key) {
        Some(c) => c.get(1).map_or(jira_key, |m| m.as_str()),
        None => jira_key,
    }
}

pub fn submit_for_review(pr: &PullRequest,
                         commits: &Vec<Commit>,
                         projects: &Vec<String>,
                         jira: &jira::api::Session,
                         config: &JiraConfig) {
    for key in get_fixed_jira_keys(commits, projects) {
        // add comment
        if let Err(e) = jira.comment_issue(&key, &format!("Review submitted for branch {}: {}", pr.base.ref_name, pr.html_url)) {
            error!("Error commenting on key [{}]: {}", key, e);
            continue; // give up on transitioning if we can't comment.
        }

        // try to transition to in-progress
        try_transition(&key, &config.progress_states(), jira);
        // try transition to pending-review
        try_transition(&key, &config.review_states(), jira);
    }

    for key in get_referenced_jira_keys(commits, projects) {
        // add comment
        if let Err(e) = jira.comment_issue(&key, &format!("Referenced by review submitted for branch {}: {}", pr.base.ref_name, pr.html_url)) {
            error!("Error commenting on key [{}]: {}", key, e);
            continue; // give up on transitioning if we can't comment.
        }

        // try to transition to in-progress
        try_transition(&key, &config.progress_states(), jira);
    }
}

pub fn resolve_issue(branch: &str,
                     version: Option<&str>,
                     commits: &Vec<PushCommit>,
                     projects: &Vec<String>,
                     jira: &jira::api::Session,
                     config: &JiraConfig) {
    for commit in commits {
        let desc = format!("[{}|{}]\n{{quote}}{}{{quote}}", Commit::short_hash(&commit), commit.html_url(), Commit::title(&commit));

        let version_desc = match version {
            None => String::new(),
            Some(v) => format!("\nIncluded in version {}", v),
        };

        let fix_msg = format!("Merged into branch {}: {}{}", branch, desc, version_desc);
        let ref_msg = format!("Referenced by commit merged into branch {}: {}{}", branch, desc, version_desc);

        for key in get_fixed_jira_keys(&vec![commit], projects) {
            if let Err(e) = jira.comment_issue(&key, &fix_msg) {
                error!("Error commenting on key [{}]: {}", key, e);
            }

            let to = config.resolved_states();
            match find_transition(&key, &to, jira) {
                Ok(Some(transition)) => {
                    let mut req = transition.new_request();

                    if let Some(ref fields) = transition.fields {
                        if let Some(ref resolution) = fields.resolution {
                            for res in &resolution.allowed_values {
                                for resolution in config.fixed_resolutions() {
                                    if res.name == resolution {
                                        req.set_resolution(&res);
                                        break;
                                    }
                                }
                                if req.fields.is_some() {
                                    break;
                                }
                            }
                            if req.fields.is_none() {
                                error!("Could not find fixed resolution in allowed values: [{:?}]!", resolution.allowed_values);
                            }
                        }
                    }

                    if let Err(e) = jira.transition_issue(&key, &req) {
                        error!("Error transitioning JIRA issue [{}] to one of [{:?}]: {}", key, to, e);
                    } else {
                        info!("Transitioned [{}] to one of [{:?}]", key, to);
                    }
                },
                Ok(None) => info!("JIRA [{}] cannot be transitioned to  any of [{:?}]", key, to),
                Err(e) => error!("{}", e),
            };
        }

        // add comment only to referenced jiras
        for key in get_referenced_jira_keys(&vec![commit], projects) {
            if let Err(e) = jira.comment_issue(&key, &ref_msg) {
                error!("Error commenting on key [{}]: {}", key, e);
            }
        }
    }
}

pub fn add_pending_version(maybe_version: Option<&str>, commits: &Vec<PushCommit>, projects: &Vec<String>, jira: &jira::api::Session) {
    if let Some(version) = maybe_version {
        for key in get_all_jira_keys(commits, projects) {
            if let Err(e) = jira.add_pending_version(&key, version) {
                error!("Error adding pending version {} to key{}: {}", version, key, e);
                continue;
            }
        }
    }
}

pub fn add_version(maybe_version: Option<&str>, commits: &Vec<PushCommit>, projects: &Vec<String>, jira: &jira::api::Session) {
    if let Some(version) = maybe_version {
        for key in get_all_jira_keys(commits, projects) {
            let proj = get_jira_project(&key);
            if let Err(e) = jira.add_version(proj, version) {
                error!("Error adding version {} to project {}: {}", version, proj, e);
                continue;
            }

            if let Err(e) = jira.assign_fix_version(&key, version) {
                error!("Error assigning version {} to key {}: {}", version, key, e);
                continue;
            }
        }
    }
}

fn try_transition(key: &str, to: &Vec<String>, jira: &jira::api::Session) {
    match find_transition(&key, to, jira) {
        Ok(Some(transition)) => {
            let req = transition.new_request();
            if let Err(e) = jira.transition_issue(&key, &req) {
                error!("Error transitioning JIRA issue [{}] to one of [{:?}]: {}", key, to, e);
            } else {
                info!("Transitioned [{}] to one of [{:?}]", key, to);
            }
        },
        Ok(None) => info!("JIRA [{}] cannot be transitioned to any of [{:?}]", key, to),
        Err(e) => error!("{}", e),
    };
}

fn find_transition(key: &str, to: &Vec<String>, jira: &jira::api::Session) -> Result<Option<Transition>, String> {
    let transitions = match jira.get_transitions(&key) {
        Ok(t) => t,
        Err(e) => return Err(format!("Error looking up JIRA transitions for key [{}]: {}", key, e)),
    };

    Ok(pick_transition(to, &transitions))
}

fn pick_transition(to: &Vec<String>, choices: &Vec<Transition>) -> Option<Transition> {
    for t in choices {
        for name in to {
            if &t.name == name || &t.to.name == name {
                return Some(t.clone())
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use jira::TransitionTo;

    #[test]
    pub fn test_get_jira_keys() {
        let projects = vec!["KEY".to_string(), "lowercase".to_string()];
        let mut commit = Commit::new();
        assert_eq!(Vec::<String>::new(), get_fixed_jira_keys(&vec![commit.clone()], &projects));
        assert_eq!(Vec::<String>::new(), get_referenced_jira_keys(&vec![commit.clone()], &projects));

        commit.commit.message = "Fix [KEY-1][KEY-2], [KEY-3] Some thing that also fixed [KEY-4] which somehow fixes KEY-5".into();
        assert_eq!(vec!["KEY-1", "KEY-2", "KEY-3", "KEY-4", "KEY-5"], get_fixed_jira_keys(&vec![commit.clone()], &projects));

        commit.commit.message += "\n\nFix: [KEY-6], and also mentions [KEY-6], [KEY-7] but not [lowercase-99]";
        assert_eq!(vec!["KEY-1", "KEY-2", "KEY-3", "KEY-4", "KEY-5", "KEY-6"], get_fixed_jira_keys(&vec![commit.clone()], &projects));
        assert_eq!(vec!["KEY-7"], get_referenced_jira_keys(&vec![commit.clone()], &projects));
    }

    #[test]
    pub fn test_get_jira_keys_alt_format() {
        let projects = vec!["KEY".to_string(), "OTHER".to_string()];
        let mut commit = Commit::new();
        commit.commit.message = "KEY-1, KEY-2:Some thing that also fixed\n\nAlso [KEY-3], OTHER-5".into();
        assert_eq!(Vec::<String>::new(), get_fixed_jira_keys(&vec![commit.clone()], &projects));
        assert_eq!(vec!["KEY-1", "KEY-2", "KEY-3", "OTHER-5"], get_referenced_jira_keys(&vec![commit.clone()], &projects));
    }

    #[test]
    pub fn test_get_jira_keys_not_allowed_project() {
        let projects = vec!["KEY".to_string()];
        let mut commit = Commit::new();
        commit.commit.message = "KEY-1, OTHER-2:Fixed stuff".into();
        assert_eq!(vec!["KEY-1"], get_referenced_jira_keys(&vec![commit.clone()], &projects));
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
        assert_eq!(Some(t1.clone()), pick_transition(&vec!["t1".into()], &vec![t1.clone(), t2.clone()]));
        assert_eq!(Some(t1.clone()), pick_transition(&vec!["inside-t1".into(), "t2".into()], &vec![t1.clone(), t2.clone()]));
        assert_eq!(Some(t2.clone()), pick_transition(&vec!["inside-t2".into()], &vec![t1.clone(), t2.clone()]));
        assert_eq!(None, pick_transition(&vec!["something-else".into()], &vec![t1.clone(), t2.clone()]));
    }

    #[test]
    fn test_get_jira_projectt() {
        assert_eq!("SERVER", get_jira_project("SERVER-123"));
        assert_eq!("BUILD", get_jira_project("BUILD"));
        assert_eq!("doesn't match", get_jira_project("doesn't match"));
    }
}
