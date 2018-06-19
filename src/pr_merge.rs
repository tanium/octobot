use std::borrow::Borrow;
use std::sync::Arc;

use regex::Regex;
use threadpool::{self, ThreadPool};

use config::Config;
use errors::*;
use git::Git;
use git_clone_manager::GitCloneManager;
use github;
use github::api::Session;
use messenger;
use slack::{SlackAttachmentBuilder, SlackRequest};
use worker::{self, WorkSender};

fn clone_and_merge_pull_request(
    session: &Session,
    clone_mgr: &GitCloneManager,
    owner: &str,
    repo: &str,
    pull_request: &github::PullRequest,
    target_branch: &str,
) -> Result<github::PullRequest> {

    let held_clone_dir = clone_mgr.clone(owner, repo)?;
    let clone_dir = held_clone_dir.dir();
    let git = Git::new(session.github_host(), session.github_token(), clone_dir);

    merge_pull_request(&git, session, owner, repo, pull_request, target_branch)
}

pub fn merge_pull_request(
    git: &Git,
    session: &Session,
    owner: &str,
    repo: &str,
    pull_request: &github::PullRequest,
    target_branch: &str,
) -> Result<github::PullRequest> {
    if !pull_request.is_merged() {
        return Err(format!("Pull Request #{} is not yet merged.", pull_request.number).into());
    }

    let merge_commit_sha;
    if let Some(ref sha) = pull_request.merge_commit_sha {
        merge_commit_sha = sha;
    } else {
        return Err(format!("Pull Request #{} has no merge commit.", pull_request.number).into());
    }

    // strip everything before last slash
    let regex = Regex::new(r".*/").unwrap();
    let pr_branch_name =
        format!("{}-{}", regex.replace(&pull_request.head.ref_name, ""), regex.replace(&target_branch, ""));

    // make sure there isn't already such a branch
    let current_remotes = git.run(&["ls-remote", "--heads"])?;
    if current_remotes.contains(&format!("refs/heads/{}", pr_branch_name)) {
        return Err(format!("PR branch already exists on origin: '{}'", pr_branch_name).into());
    }

    let (title, body, whitespace_mode) = cherry_pick(
        &git,
        &merge_commit_sha,
        &pr_branch_name,
        pull_request.number,
        &target_branch,
        &pull_request.base.ref_name,
    )?;

    git.run(&["push", "origin", &format!("HEAD:{}", pr_branch_name)])?;

    let new_pr = session.create_pull_request(owner, repo, &title, &body, &pr_branch_name, &target_branch)?;

    let assignees: Vec<String> = pull_request.assignees.iter().map(|a| a.login().to_string()).collect();
    session.assign_pull_request(owner, repo, new_pr.number, assignees)?;

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
) -> Result<(String, String, String)> {
    git.checkout_branch(pr_branch_name, &format!("origin/{}", target_branch))?;

    // cherry-pick!

    let mut whitespace_mode = "";
    if let Err(e) = do_cherry_pick(git, &commit_hash, &[]) {
        info!("Could not cherry-pick normally. Ignoring changed whitespace. {}", e);

        whitespace_mode = "ignore-space-change";
        if let Err(e) = do_cherry_pick(git, &commit_hash, &["-X", whitespace_mode]) {
            info!("Could not cherry-pick with `-X {}`. Ignoring all whitespace. {}", whitespace_mode, e);

            whitespace_mode = "ignore-all-space";
            if let Err(e) = do_cherry_pick(git, &commit_hash, &["-X", whitespace_mode]) {
                info!("Could not cherry-pick with `-X {}`: {}", whitespace_mode, e);
                return Err(e);
            }
        }
    }

    let desc = git.get_commit_desc(commit_hash)?;
    let (title, body) = make_merge_desc(desc, commit_hash, pr_number, target_branch, orig_base_branch);

    // change commit message
    git.run_with_stdin(&["commit", "--amend", "-F", "-"], &format!("{}\n\n{}", &title, &body))?;

    Ok((title, body, whitespace_mode.into()))
}

fn do_cherry_pick(git: &Git, commit_hash: &str, opts: &[&str]) -> Result<String> {
    git.run(&vec!["reset", "--hard"])?;

    let mut args = vec!["-c", "merge.renameLimit=999999", "cherry-pick", "--allow-empty"];

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
) -> (String, String) {
    // grab original title and strip out the PR number at the end
    let pr_regex = Regex::new(r"(\s*\(#\d+\))+$").unwrap();
    let orig_title = pr_regex.replace(&orig_desc.0, "");
    // strip out 'release' from the prefix to keep titles shorter
    let release_branch_regex = Regex::new(r"^release/").unwrap();
    let title = format!("{}->{}: {}", orig_base_branch, release_branch_regex.replace(target_branch, ""), orig_title);
    let mut body = orig_desc.1;

    if body.len() != 0 {
        body += "\n\n";
    }
    body += format!("(cherry-picked from {}, PR #{})", commit_hash, pr_number).as_str();

    (title, body)
}

#[derive(Debug)]
pub struct PRMergeRequest {
    pub repo: github::Repo,
    pub pull_request: github::PullRequest,
    pub target_branch: String,
}

struct Runner {
    config: Arc<Config>,
    github_session: Arc<Session>,
    clone_mgr: Arc<GitCloneManager>,
    slack: WorkSender<SlackRequest>,
    thread_pool: ThreadPool,
}

pub fn req(repo: &github::Repo, pull_request: &github::PullRequest, target_branch: &str) -> PRMergeRequest {
    PRMergeRequest {
        repo: repo.clone(),
        pull_request: pull_request.clone(),
        target_branch: target_branch.to_string(),
    }
}

pub fn new_worker(
    max_concurrency: usize,
    config: Arc<Config>,
    github_session: Arc<Session>,
    clone_mgr: Arc<GitCloneManager>,
    slack: WorkSender<SlackRequest>,
) -> worker::Worker<PRMergeRequest> {
    worker::Worker::new(
        "pr-merge",
        Runner {
            config: config,
            github_session: github_session,
            clone_mgr: clone_mgr.clone(),
            slack: slack,
            thread_pool: threadpool::Builder::new()
                .num_threads(max_concurrency)
                .thread_name("pr-merge".to_string())
                .build(),
        },
    )
}

impl worker::Runner<PRMergeRequest> for Runner {
    fn handle(&self, req: PRMergeRequest) {
        let github_session = self.github_session.clone();
        let clone_mgr = self.clone_mgr.clone();
        let config = self.config.clone();

        let slack = self.slack.clone();

        // launch another thread to do the merge
        self.thread_pool.execute(move || {
            let merge_result = clone_and_merge_pull_request(
                github_session.borrow(),
                &clone_mgr,
                &req.repo.owner.login(),
                &req.repo.name,
                &req.pull_request,
                &req.target_branch,
            );
            if let Err(e) = merge_result {

                let attach = SlackAttachmentBuilder::new(&format!("{}", e))
                    .title(
                        format!("Source PR: #{}: \"{}\"", req.pull_request.number, req.pull_request.title)
                            .as_str(),
                    )
                    .title_link(req.pull_request.html_url.clone())
                    .color("danger")
                    .build();

                let messenger = messenger::new(config, slack);
                messenger.send_to_owner(
                    &format!(
                        "Error creating merge PR from {} to {}",
                        req.pull_request.head.ref_name,
                        req.target_branch
                    ),
                    &vec![attach],
                    &req.pull_request.user,
                    &req.repo,
                );
            }
        });
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
            "release/target_branch",
            "source_branch",
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
        );

        assert_eq!(desc.0, "source_branch->other_branch: Yay, I made a change");
        assert_eq!(desc.1, "(cherry-picked from abcdef, PR #99)");
    }
}
