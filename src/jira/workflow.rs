use regex::Regex;

use config::JiraConfig;
use github::PullRequest;
use jira;
use jira::Transition;

fn get_jira_keys(pr: &PullRequest) -> Vec<String> {
    let re = Regex::new(r"\[([A-Z]+-[0-9]+)\]").unwrap();

    // TODO: should references to jiras in the PR body count the same?
    let title_and_body = pr.title.clone();

    re.captures_iter(&title_and_body).map(|c| c[1].to_string()).collect()
}

pub fn submit_for_review(pr: &PullRequest, jira: &jira::api::Session, config: &JiraConfig) {
    for key in get_jira_keys(pr) {
        // add comment
        if let Err(e) = jira.comment_issue(&key, &format!("Review submitted for branch {} ({}): {}", pr.base.ref_name, pr.head.ref_name, pr.html_url)) {
            error!("Error commenting on key [{}]: {}", key, e);
            continue; // give up on transitioning if we can't comment.
        }

        // try to transition to in-progress
        try_transition(&key, &config.progress_states(), jira);
        // try transition to pending-review
        try_transition(&key, &config.review_states(), jira);
    }
}

pub fn resolve_issue(pr: &PullRequest, jira: &jira::api::Session, config: &JiraConfig) {
    for key in get_jira_keys(pr) {
        let pr_desc;
        if pr.body.trim().len() == 0 {
            pr_desc = pr.title.clone();
        } else {
            pr_desc = format!("{}\n\n{}", pr.title, pr.body);
        }

        // add comment
        let msg = format!("Merged into branch {}: {}\n\n{{quote}}{}{{quote}}", pr.base.ref_name, pr.html_url, pr_desc);
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
        let mut pr = PullRequest::new();
        assert_eq!(Vec::<String>::new(), get_jira_keys(&pr));

        pr.title = "[KEY-1][KEY-2] Some thing that also fixed [KEY-3]".into();
        assert_eq!(vec!["KEY-1", "KEY-2", "KEY-3"], get_jira_keys(&pr));

        pr.body = "Oh, I forgot it also fixes [KEY-4]".into();
        assert_eq!(vec!["KEY-1", "KEY-2", "KEY-3"], get_jira_keys(&pr));
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
