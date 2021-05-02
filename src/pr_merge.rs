use std::borrow::Borrow;
use std::sync::Arc;

use conventional::{Commit, Simple as _};
use failure::format_err;
use log::{error, info};
use regex::Regex;

use crate::config::Config;
use crate::errors::*;
use crate::git::Git;
use crate::git_clone_manager::GitCloneManager;
use crate::github;
use crate::github::api::{GithubSessionFactory, Session};
use crate::messenger;
use crate::slack::{SlackAttachmentBuilder, SlackRequest};
use crate::worker;

fn clone_and_merge_pull_request(
    github_app: &dyn GithubSessionFactory,
    clone_mgr: &GitCloneManager,
    req: &PRMergeRequest,
    config: Arc<Config>,
    slack: Arc<dyn worker::Worker<SlackRequest>>,
) {
    let owner = &req.repo.owner.login();
    let repo = &req.repo.name;

    let session = match github_app.new_session(owner, repo) {
        Ok(s) => s,
        Err(e) => {
            error!("Error getting new session: {}", e);
            return;
        }
    };
    let held_clone_dir = match clone_mgr.clone(owner, repo) {
        Ok(h) => h,
        Err(e) => {
            error!("Error getting new session: {}", e);
            return;
        }
    };
    let clone_dir = held_clone_dir.dir();
    let git = Git::new(session.github_host(), session.github_token(), clone_dir);

    merge_pull_request(&git, &session, &req, config, slack)
}

pub fn merge_pull_request(
    git: &Git,
    session: &dyn Session,
    req: &PRMergeRequest,
    config: Arc<Config>,
    slack: Arc<dyn worker::Worker<SlackRequest>>,
) {
    if let Err(e) = try_merge_pull_request(git, session, req) {
        let attach = SlackAttachmentBuilder::new(&format!("{}", e))
            .title(
                format!("Source PR: #{}: \"{}\"", req.pull_request.number, req.pull_request.title)
                    .as_str(),
            )
            .title_link(req.pull_request.html_url.clone())
            .color("danger")
            .build();

        let messenger = messenger::new(config.clone(), slack.clone());
        messenger.send_to_owner(
            &format!(
                "Error creating merge PR from {} to {}",
                req.pull_request.head.ref_name,
                req.target_branch
            ),
            &vec![attach],
            &req.pull_request.user,
            &req.repo,
            &req.target_branch,
            &req.commits,
        );
    }
}

pub fn try_merge_pull_request(
    git: &Git,
    session: &dyn Session,
    req: &PRMergeRequest,
) -> Result<github::PullRequest> {
    let pull_request = &req.pull_request;
    if !pull_request.is_merged() {
        return Err(format_err!("Pull Request #{} is not yet merged.", pull_request.number));
    }

    let merge_commit_sha;
    if let Some(ref sha) = pull_request.merge_commit_sha {
        merge_commit_sha = sha;
    } else {
        return Err(format_err!("Pull Request #{} has no merge commit.", pull_request.number));
    }

    // strip everything before last slash
    let regex = Regex::new(r".*/").unwrap();
    let pr_branch_name =
        format!("{}-{}", regex.replace(&pull_request.head.ref_name, ""), regex.replace(&req.target_branch, ""));

    // make sure there isn't already such a branch
    let current_remotes = git.run(&["ls-remote", "--heads"])?;
    if current_remotes.contains(&format!("refs/heads/{}", pr_branch_name)) {
        return Err(format_err!("PR branch already exists on origin: '{}'", pr_branch_name));
    }

    let (title, body, whitespace_mode) = cherry_pick(
        &git,
        &merge_commit_sha,
        &pr_branch_name,
        pull_request.number,
        &req.target_branch,
        &pull_request.base.ref_name,
        &req.release_branch_prefix,
    )?;

    git.run(&["push", "origin", &format!("HEAD:{}", pr_branch_name)])?;

    let owner = &req.repo.owner.login();
    let repo = &req.repo.name;
    let new_pr = session.create_pull_request(owner, repo, &title, &body, &pr_branch_name, &req.target_branch)?;

    let mut assignees: Vec<String> = pull_request.assignees.iter().map(|a| a.login().to_string()).collect();
    assignees.retain(|r| r != pull_request.user.login());
    if !assignees.is_empty() {
        session.assign_pull_request(owner, repo, new_pr.number, assignees)?;
    }

    let mut reviewers: Vec<String> = pull_request.all_reviewers().into_iter().map(|a| a.login().to_string()).collect();
    reviewers.retain(|r| r != pull_request.user.login());
    if !reviewers.is_empty() {
        session.request_review(owner, repo, new_pr.number, reviewers)?;
    }

    if whitespace_mode.len() > 0 {
        let msg = format!("Cherry-pick required option `{}`. Please verify correctness.", whitespace_mode);
        if let Err(e) = session.comment_pull_request(owner, repo, new_pr.number, &msg) {
            error!("Error making whitespace comment on pull request: {}", e);
        }
    }

    Ok(new_pr)
}

pub fn cherry_pick(
    git: &Git,
    commit_hash: &str,
    pr_branch_name: &str,
    pr_number: u32,
    target_branch: &str,
    orig_base_branch: &str,
    release_branch_prefix: &str,
) -> Result<(String, String, String)> {
    git.checkout_branch(pr_branch_name, &format!("origin/{}", target_branch))?;

    let (user, email) = git.get_commit_author(commit_hash)?;
    let email = format!("user.email={}", email);
    let user = format!("user.name={}", user);
    let user_opts = ["-c", &email, "-c", &user];

    // cherry-pick!

    let mut whitespace_mode = "";
    if let Err(e) = do_cherry_pick(git, &commit_hash, &[], &user_opts) {
        info!("Could not cherry-pick normally. Ignoring changed whitespace. {}", e);

        whitespace_mode = "ignore-space-change";
        if let Err(e) = do_cherry_pick(git, &commit_hash, &["-X", whitespace_mode], &user_opts) {
            info!("Could not cherry-pick with `-X {}`. Ignoring all whitespace. {}", whitespace_mode, e);

            whitespace_mode = "ignore-all-space";
            if let Err(e) = do_cherry_pick(git, &commit_hash, &["-X", whitespace_mode], &user_opts) {
                info!("Could not cherry-pick with `-X {}`: {}", whitespace_mode, e);
                return Err(e);
            }
        }
    }

    let desc = git.get_commit_desc(commit_hash)?;
    let (title, body) = make_merge_desc(desc, commit_hash, pr_number, target_branch, orig_base_branch, release_branch_prefix);

    // change commit message
    let mut amend_args = vec![];
    amend_args.extend(user_opts.iter());
    amend_args.extend(["commit", "--amend", "-F", "-"].iter());
    git.run_with_stdin(&amend_args, &format!("{}\n\n{}", &title, &body))?;

    Ok((title, body, whitespace_mode.into()))
}

fn do_cherry_pick(git: &Git, commit_hash: &str, opts: &[&str], user_opts: &[&str]) -> Result<String> {
    git.run(&vec!["reset", "--hard"])?;

    let mut args = vec!["-c", "merge.renameLimit=999999"];
    args.extend(user_opts.iter());
    args.extend(vec!["cherry-pick", "--allow-empty"].iter());

    for i in 0..opts.len() {
        args.push(opts[i]);
    }
    args.push(commit_hash);

    git.run(&args)
}

fn make_merge_desc(
    orig_desc: (String, String),
    commit_hash: &str,
    pr_number: u32,
    target_branch: &str,
    orig_base_branch: &str,
    release_branch_prefix: &str,
) -> (String, String) {
    // grab original title and strip out the PR number at the end
    let pr_regex = Regex::new(r"(\s*\(#\d+\))+$").unwrap();
    let prev_merge_regex = Regex::new(r"^([^:]+->[^:]+: )+").unwrap();

    // strip out PR from title
    let orig_title = pr_regex.replace(&orig_desc.0, "");
    // strip out previous merge title prefixes
    let mut orig_title = prev_merge_regex.replace(&orig_title, "").to_owned().to_string();

    // strip out conventional commit prefix
    let mut prefix = String::new();
    match Commit::new(&orig_title) {
        Ok(commit) => {
            prefix = commit.type_().to_owned();
            if let Some(s) = commit.scope() {
                prefix += &format!("({})", s);
            }
            if commit.breaking() {
                prefix += "!";
            }
            prefix += ": ";
            orig_title = commit.description().to_owned();
        }
        Err(_) => (),
    };

    // strip out 'release' from the prefix to keep titles shorter
    let mut target_branch = target_branch.to_owned();
    if target_branch.starts_with(release_branch_prefix) {
        target_branch = target_branch.replacen(release_branch_prefix, "", 1);
    }
    let mut orig_base_branch = orig_base_branch.to_owned();
    if orig_base_branch.starts_with(release_branch_prefix) {
        orig_base_branch = orig_base_branch.replacen(release_branch_prefix, "", 1);
    }

    let title = format!("{}{}->{}: {}", prefix, orig_base_branch, target_branch, orig_title);
    let mut body = orig_desc.1;

    if body.len() != 0 {
        body += "\n\n";
    }
    body += format!("(cherry-picked from {}, PR #{})", commit_hash, pr_number).as_str();

    (title, body)
}

#[derive(Debug, PartialEq)]
pub struct PRMergeRequest {
    pub repo: github::Repo,
    pub pull_request: github::PullRequest,
    pub target_branch: String,
    pub release_branch_prefix: String,
    pub commits: Vec<github::Commit>,
}

struct Runner {
    config: Arc<Config>,
    github_app: Arc<dyn GithubSessionFactory>,
    clone_mgr: Arc<GitCloneManager>,
    slack: Arc<dyn worker::Worker<SlackRequest>>,
}

pub fn req(repo: &github::Repo, pull_request: &github::PullRequest, target_branch: &str, release_branch_prefix: &str, commits: Vec<github::Commit>) -> PRMergeRequest {
    PRMergeRequest {
        repo: repo.clone(),
        pull_request: pull_request.clone(),
        target_branch: target_branch.to_string(),
        release_branch_prefix: release_branch_prefix.to_string(),
        commits: commits,
    }
}

pub fn new_runner(
    config: Arc<Config>,
    github_app: Arc<dyn GithubSessionFactory>,
    clone_mgr: Arc<GitCloneManager>,
    slack: Arc<dyn worker::Worker<SlackRequest>>,
) -> Arc<dyn worker::Runner<PRMergeRequest>> {
    Arc::new(Runner {
        config: config,
        github_app: github_app,
        clone_mgr: clone_mgr.clone(),
        slack: slack,
    })
}

impl worker::Runner<PRMergeRequest> for Runner {
    fn handle(&self, req: PRMergeRequest) {
        clone_and_merge_pull_request(
            self.github_app.borrow(),
            self.clone_mgr.borrow(),
            &req,
            self.config.clone(),
            self.slack.clone(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_make_merge_desc() {
        let desc = make_merge_desc(
            (String::from("Yay, I made a change (#99)"), String::from("here is more data about it")),
            "abcdef",
            99,
            "release/target_branch",
            "source_branch",
            "release/",
        );

        assert_eq!(desc.0, "source_branch->target_branch: Yay, I made a change");
        assert_eq!(desc.1, "here is more data about it\n\n(cherry-picked from abcdef, PR #99)");
    }

    #[test]
    fn test_make_merge_desc_no_body() {
        let desc = make_merge_desc(
            (String::from("Yay, I made a change (#99)"), String::from("")),
            "abcdef",
            99,
            "the-release-target_branch",
            "source_branch",
            "the-release-",
        );

        assert_eq!(desc.0, "source_branch->target_branch: Yay, I made a change");
        assert_eq!(desc.1, "(cherry-picked from abcdef, PR #99)");
    }

    #[test]
    fn test_make_merge_desc_no_release_branch() {
        let desc = make_merge_desc(
            (String::from("Yay, I made a change (#99)"), String::from("")),
            "abcdef",
            99,
            "other_branch",
            "source_branch",
            "release/",
        );

        assert_eq!(desc.0, "source_branch->other_branch: Yay, I made a change");
        assert_eq!(desc.1, "(cherry-picked from abcdef, PR #99)");
    }

    #[test]
    fn test_make_merge_desc_from_release_branch() {
        let desc = make_merge_desc(
            (String::from("Yay, I made a change (#99)"), String::from("")),
            "abcdef",
            99,
            "release-other_branch",
            "release-source_branch",
            "release-",
        );

        assert_eq!(desc.0, "source_branch->other_branch: Yay, I made a change");
        assert_eq!(desc.1, "(cherry-picked from abcdef, PR #99)");
    }

    #[test]
    fn test_make_merge_desc_multi1() {
        let desc = make_merge_desc(
            (String::from("prev_branch->source_branch: Yay, I made a change (#99)"), String::from("")),
            "abcdef",
            99,
            "other_branch",
            "source_branch",
            "release/",
        );

        assert_eq!(desc.0, "source_branch->other_branch: Yay, I made a change");
        assert_eq!(desc.1, "(cherry-picked from abcdef, PR #99)");
    }

    #[test]
    fn test_make_merge_desc_multi2() {
        let desc = make_merge_desc(
            (String::from("prev_branch->source_branch: more_branches->prev_branch: Yay, I made a change (#99)"), String::from("")),
            "abcdef",
            99,
            "other_branch",
            "source_branch",
            "release/",
        );

        assert_eq!(desc.0, "source_branch->other_branch: Yay, I made a change");
        assert_eq!(desc.1, "(cherry-picked from abcdef, PR #99)");
    }
}
