use std::borrow::Borrow;
use std::sync::Arc;

use regex::Regex;
use threadpool::ThreadPool;

use config::Config;
use git::Git;
use github;
use github::api::Session;
use git_clone_manager::GitCloneManager;
use messenger;
use slack::SlackAttachmentBuilder;
use worker;

pub fn merge_pull_request(session: &Session, clone_mgr: &GitCloneManager, owner: &str, repo: &str,
                          pull_request: &github::PullRequest, target_branch: &str)
                          -> Result<github::PullRequest, String> {
    Merger::new(session, clone_mgr).merge_pull_request(owner, repo, pull_request, target_branch)
}


struct Merger<'a> {
    session: &'a Session,
    clone_mgr: &'a GitCloneManager,
}

impl<'a> Merger<'a> {
    pub fn new(session: &'a Session, clone_mgr: &'a GitCloneManager) -> Merger<'a> {
        Merger {
            session: session,
            clone_mgr: clone_mgr,
        }
    }

    pub fn merge_pull_request(&self, owner: &str, repo: &str, pull_request: &github::PullRequest,
                              target_branch: &str)
                              -> Result<github::PullRequest, String> {
        if !pull_request.is_merged() {
            return Err(format!("Pull Request #{} is not yet merged.", pull_request.number));
        }

        let merge_commit_sha;
        if let Some(ref sha) = pull_request.merge_commit_sha {
            merge_commit_sha = sha;
        } else {
            return Err(format!("Pull Request #{} has no merge commit.", pull_request.number));
        }

        // strip everything before last slash
        let regex = Regex::new(r".*/").unwrap();
        let pr_branch_name = format!("{}-{}",
                                     regex.replace(&pull_request.head.ref_name, ""),
                                     regex.replace(&target_branch, ""));

        let held_clone_dir = try!(self.clone_mgr.clone(owner, repo));
        let clone_dir = held_clone_dir.dir();

        let git = Git::new(self.session.github_host(), self.session.github_token(), clone_dir);

        // make sure there isn't already such a branch
        let current_remotes = try!(git.run(&["ls-remote", "--heads"]));
        if current_remotes.contains(&format!("refs/heads/{}", pr_branch_name)) {
            return Err(format!("PR branch already exists on origin: '{}'", pr_branch_name));
        }

        let (title, body) = try!(self.cherry_pick(&git,
                                                  &merge_commit_sha,
                                                  &pr_branch_name,
                                                  pull_request.number,
                                                  &target_branch,
                                                  &pull_request.base.ref_name));

        try!(git.run(&["push", "origin", &format!("{}:{}", pr_branch_name, pr_branch_name)]));

        let new_pr = try!(self.session
            .create_pull_request(owner, repo, &title, &body, &pr_branch_name, &target_branch));

        let assignees: Vec<String> =
            pull_request.assignees.iter().map(|a| a.login().to_string()).collect();
        try!(self.session.assign_pull_request(owner, repo, new_pr.number, assignees));

        Ok(new_pr)
    }

    fn cherry_pick(&self, git: &Git, commit_hash: &str, pr_branch_name: &str,
                   pr_number: u32, target_branch: &str, orig_base_branch: &str)
                   -> Result<(String, String), String> {
        try!(git.checkout_branch(pr_branch_name, &format!("origin/{}", target_branch)));

        // cherry-pick!
        try!(git.run(&["-c", "merge.renameLimit=999999", "cherry-pick", "-X", "ignore-all-space", commit_hash]));

        let desc = try!(git.get_commit_desc(commit_hash));
        let desc = make_merge_desc(desc, commit_hash, pr_number, target_branch, orig_base_branch);

        // change commit message
        try!(git.run_with_stdin(&["commit", "--amend", "-F", "-"], &format!("{}\n\n{}", &desc.0, &desc.1)));

        Ok(desc)
    }
}

fn make_merge_desc(orig_desc: (String, String), commit_hash: &str, pr_number: u32,
                   target_branch: &str, orig_base_branch: &str) -> (String, String) {
    // grab original title and strip out the PR number at the end
    let pr_regex = Regex::new(r"(\s*\(#\d+\))+$").unwrap();
    let orig_title = pr_regex.replace(&orig_desc.0, "");
    // strip out 'release' from the prefix to keep titles shorter
    let release_branch_regex = Regex::new(r"^release/").unwrap();
    let title = format!("{}->{}: {}",
                        orig_base_branch,
                        release_branch_regex.replace(target_branch, ""),
                        orig_title);
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
    thread_pool: ThreadPool,
}

pub fn req(repo: &github::Repo, pull_request: &github::PullRequest, target_branch: &str) -> PRMergeRequest {
    PRMergeRequest {
        repo: repo.clone(),
        pull_request: pull_request.clone(),
        target_branch: target_branch.to_string(),
    }
}

pub fn new_worker(max_concurrency: usize, config: Arc<Config>, github_session: Arc<Session>, clone_mgr: Arc<GitCloneManager>)
               -> worker::Worker<PRMergeRequest> {
    worker::Worker::new(Runner {
        config: config,
        github_session: github_session,
        clone_mgr: clone_mgr.clone(),
        thread_pool: ThreadPool::new(max_concurrency),
    })
}

impl worker::Runner<PRMergeRequest> for Runner {
    fn handle(&self, req: PRMergeRequest) {
        let github_session = self.github_session.clone();
        let clone_mgr = self.clone_mgr.clone();
        let config = self.config.clone();

        // launch another thread to do the merge
        self.thread_pool.execute(move || {
            if let Err(e) = merge_pull_request(github_session.borrow(),
                                               &clone_mgr,
                                               &req.repo.owner.login(),
                                               &req.repo.name,
                                               &req.pull_request,
                                               &req.target_branch) {

                let attach = SlackAttachmentBuilder::new(&e)
                    .title(format!("Source PR: #{}: \"{}\"",
                                   req.pull_request.number,
                                   req.pull_request.title)
                        .as_str())
                    .title_link(req.pull_request.html_url.clone())
                    .color("danger")
                    .build();

                let messenger = messenger::from_config(config);
                messenger.send_to_owner("Error creating merge Pull Request",
                                        &vec![attach],
                                        &req.pull_request.user,
                                        &req.repo);
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
            "abcdef", 99, "release/target_branch", "source_branch");

        assert_eq!(desc.0, "source_branch->target_branch: Yay, I made a change");
        assert_eq!(desc.1, "here is more data about it\n\n(cherry-picked from abcdef, PR #99)");
    }

    #[test]
    fn test_make_merge_desc_no_body() {
        let desc = make_merge_desc(
            (String::from("Yay, I made a change (#99)"), String::from("")),
            "abcdef", 99, "release/target_branch", "source_branch");

        assert_eq!(desc.0, "source_branch->target_branch: Yay, I made a change");
        assert_eq!(desc.1, "(cherry-picked from abcdef, PR #99)");
    }

    #[test]
    fn test_make_merge_desc_no_release_branch() {
        let desc = make_merge_desc(
            (String::from("Yay, I made a change (#99)"), String::from("")),
            "abcdef", 99, "other_branch", "source_branch");

        assert_eq!(desc.0, "source_branch->other_branch: Yay, I made a change");
        assert_eq!(desc.1, "(cherry-picked from abcdef, PR #99)");
    }
}
