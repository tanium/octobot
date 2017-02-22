use regex::Regex;

use config::JiraConfig;
use github::{Commit, CommitLike, PullRequest, PushCommit};
use jira;
use jira::Transition;

fn get_jira_keys(strings: Vec<String>) -> Vec<String> {
    let re = Regex::new(r"\[([A-Z]+-[0-9]+)\]").unwrap();

    let mut all_keys = vec![];
    for s in strings {
        // TODO: should references to jiras in the commit body count the same?
        all_keys.extend(re.captures_iter(&s).map(|c| c[1].to_string()));
    }

    all_keys.sort();
    all_keys.dedup();

    all_keys
}

fn get_fixed_jira_keys<T: CommitLike>(commits: &Vec<T>) -> Vec<String> {
    get_jira_keys(commits.iter().map(|c| Commit::title(c)).collect())
}

fn get_referenced_jira_keys<T: CommitLike>(commits: &Vec<T>) -> Vec<String> {
    get_jira_keys(commits.iter().map(|c| Commit::body(c)).collect())
}

pub fn submit_for_review(pr: &PullRequest, commits: &Vec<Commit>, jira: &jira::api::Session, config: &JiraConfig) {
    for key in get_fixed_jira_keys(commits) {
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
}

pub fn resolve_issue(prs: &Vec<PullRequest>, commits: &Vec<PushCommit>, jira: &jira::api::Session, config: &JiraConfig) {
    for commit in commits {
        let desc = format!("[{}|{}]\n{{quote}}{}{{quote}}", Commit::short_hash(&commit), commit.html_url(), commit.message());

        for key in get_fixed_jira_keys(&vec![commit]) {
            let mut msg = desc.clone();
            for pr in prs {
                msg += &format!("\nMerged into branch {}: {}", pr.base.ref_name, pr.html_url);
            }

            if let Err(e) = jira.comment_issue(&key, &msg) {
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
        for key in get_referenced_jira_keys(&vec![commit]) {
            let mut msg = desc.clone();
            for pr in prs {
                msg += &format!("\nReferenced by commit merged into branch {}: {}", pr.base.ref_name, pr.html_url);
            }
            if let Err(e) = jira.comment_issue(&key, &msg) {
                error!("Error commenting on key [{}]: {}", key, e);
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
        let mut commit = Commit::new();
        assert_eq!(Vec::<String>::new(), get_fixed_jira_keys(&vec![commit.clone()]));
        assert_eq!(Vec::<String>::new(), get_referenced_jira_keys(&vec![commit.clone()]));

        // should include the commit message title
        commit.commit.message = "[KEY-1][KEY-2] Some thing that also fixed [KEY-3][KEY-3]".into();
        assert_eq!(vec!["KEY-1", "KEY-2", "KEY-3"], get_fixed_jira_keys(&vec![commit.clone()]));
        assert_eq!(Vec::<String>::new(), get_referenced_jira_keys(&vec![commit.clone()]));

        // should ignore the commit message body.
        commit.commit.message += "\n\nOh, I forgot it also fixes [KEY-4]";
        assert_eq!(vec!["KEY-1", "KEY-2", "KEY-3"], get_fixed_jira_keys(&vec![commit.clone()]));
        assert_eq!(vec!["KEY-4"], get_referenced_jira_keys(&vec![commit.clone()]));
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

}
