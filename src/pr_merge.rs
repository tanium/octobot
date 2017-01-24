use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread::{self, JoinHandle};

use regex::Regex;
use threadpool::ThreadPool;

use config::Config;
use dir_pool::DirPool;
use git::Git;
use github;
use github::api::Session;
use messenger;
use slack::SlackAttachmentBuilder;

pub fn merge_pull_request(session: &Session, dir_pool: &DirPool, owner: &str, repo: &str,
                          pull_request: &github::PullRequest, target_branch: &str)
                          -> Result<github::PullRequest, String> {
    Merger::new(session, dir_pool).merge_pull_request(owner, repo, pull_request, target_branch)
}


struct Merger<'a> {
    git: Git,
    session: &'a Session,
    dir_pool: &'a DirPool,
}

impl<'a> Merger<'a> {
    pub fn new(session: &'a Session, dir_pool: &'a DirPool) -> Merger<'a> {
        Merger {
            git: Git::new(session.github_host(), session.github_token()),
            session: session,
            dir_pool: dir_pool,
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

        let held_clone_dir = self.dir_pool.take_directory(self.session.github_host(), owner, repo);
        let clone_dir = held_clone_dir.dir();
        try!(self.clone_repo(owner, repo, &clone_dir));

        // make sure there isn't already such a branch
        let current_remotes = try!(self.git.run(&["ls-remote", "--heads"], &clone_dir));
        if current_remotes.contains(&format!("refs/heads/{}", pr_branch_name)) {
            return Err(format!("PR branch already exists on origin: '{}'", pr_branch_name));
        }

        let (title, body) = try!(self.cherry_pick(&clone_dir,
                                                  &merge_commit_sha,
                                                  &pr_branch_name,
                                                  pull_request.number,
                                                  &target_branch,
                                                  &pull_request.base.ref_name));

        try!(self.git
            .run(&["push", "origin", &format!("{}:{}", pr_branch_name, pr_branch_name)],
                 &clone_dir));

        let new_pr = try!(self.session
            .create_pull_request(owner, repo, &title, &body, &pr_branch_name, &target_branch));

        let assignees: Vec<String> =
            pull_request.assignees.iter().map(|a| a.login().to_string()).collect();
        try!(self.session.assign_pull_request(owner, repo, new_pr.number, assignees));

        Ok(new_pr)
    }

    fn clone_repo(&self, owner: &str, repo: &str, clone_dir: &PathBuf) -> Result<(), String> {
        let url = format!("https://{}@{}/{}/{}",
                          self.session.user().login(),
                          self.session.github_host(),
                          owner,
                          repo);

        if clone_dir.join(".git").exists() {
            try!(self.git.run(&["fetch"], clone_dir));
        } else {
            if let Err(e) = fs::create_dir_all(&clone_dir) {
                return Err(format!("Error creating clone directory '{:?}': {}", clone_dir, e));
            }
            try!(self.git.run(&["clone", &url, "."], clone_dir));
        }

        Ok(())
    }

    fn cherry_pick(&self, clone_dir: &PathBuf, commit_hash: &str, pr_branch_name: &str,
                   pr_number: u32, target_branch: &str, orig_base_branch: &str)
                   -> Result<(String, String), String> {
        let real_target_branch = format!("origin/{}", target_branch);

        // clean up state
        try!(self.git.run(&["reset", "--hard"], clone_dir));
        try!(self.git.run(&["clean", "-fdx"], clone_dir));

        // setup branch
        let current_branch = try!(self.git.run(&["rev-parse", "--abbrev-ref", "HEAD"], clone_dir));
        if current_branch == pr_branch_name {
            try!(self.git.run(&["reset", "--hard", &real_target_branch], clone_dir));
        } else {
            // delete if it exists
            let has_branch = try!(self.git.has_branch(pr_branch_name, clone_dir));
            if has_branch {
                try!(self.git.run(&["branch", "-D", pr_branch_name], clone_dir));
            }
            // recreate branch
            try!(self.git.run(&["checkout", "-b", pr_branch_name, &real_target_branch],
                              clone_dir));
        }

        // cherry-pick!
        try!(self.git.run(&["cherry-pick", "-X", "ignore-all-space", commit_hash],
                          clone_dir));

        let desc = try!(self.get_commit_desc(clone_dir, commit_hash));

        // grab original title and strip out the PR number at the end
        let pr_regex = Regex::new(r"(\s*\(#\d+\))+$").unwrap();
        let orig_title = pr_regex.replace(&desc.0, "");
        // strip out 'release' from the prefix to keep titles shorter
        let release_branch_regex = Regex::new(r"^release/").unwrap();
        let title = format!("{}->{}: {}",
                            orig_base_branch,
                            release_branch_regex.replace(target_branch, ""),
                            orig_title);
        let mut body = desc.1;

        if body.len() != 0 {
            body += "\n\n";
        }
        body += format!("(cherry-picked from {}, PR #{})",
                        &commit_hash[0..7],
                        pr_number)
            .as_str();

        // change commit message
        try!(self.git.run_with_stdin(&["commit", "--amend", "-F", "-"],
                                     clone_dir,
                                     &format!("{}\n\n{}", title, body)));

        Ok((title, body))
    }

    // returns (title, body)
    fn get_commit_desc(&self, clone_dir: &PathBuf, commit_hash: &str)
                       -> Result<(String, String), String> {
        let lines: Vec<String> = try!(self.git
                .run(&["log", "-1", "--pretty=%B", commit_hash], clone_dir))
            .split("\n")
            .map(|l| l.trim().to_string())
            .collect();

        if lines.len() == 0 {
            return Err(format!("Empty commit message found!"));
        }

        let title = lines[0].clone();

        let mut body = String::new();
        // skip the blank line
        if lines.len() > 2 {
            body = lines[2..].join("\n");
            body += "\n";
        }

        Ok((title, body))
    }
}

pub enum PRMergeMessage {
    Stop,
    Merge {
        repo: github::Repo,
        pull_request: github::PullRequest,
        target_branch: String,
    },
}

pub struct Worker {
    sender: Mutex<Sender<PRMergeMessage>>,
    handle: Option<JoinHandle<()>>,
}

impl PRMergeMessage {
    pub fn merge(repo: &github::Repo, pull_request: &github::PullRequest, target_branch: &str)
                 -> PRMergeMessage {
        PRMergeMessage::Merge {
            repo: repo.clone(),
            pull_request: pull_request.clone(),
            target_branch: target_branch.to_string(),
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let sender = self.new_sender();
        match sender.send(PRMergeMessage::Stop) {
            Ok(_) => {
                match self.handle.take().unwrap().join() {
                    Ok(_) => (),
                    Err(e) => error!("Error shutting down worker: {:?}", e),
                }
            }
            Err(e) => error!("Error sending stop message: {}", e),
        }
    }
}

impl Worker {
    pub fn new(max_concurrency: usize, config: Arc<Config>, github_session: Arc<Session>)
               -> Worker {

        let (tx, rx) = channel();

        let clone_root_dir = config.clone_root_dir.to_string();

        Worker {
            sender: Mutex::new(tx),
            handle: Some(thread::spawn(move || {
                let runner = WorkerRunner {
                    rx: rx,
                    config: config,
                    github_session: github_session,
                    dir_pool: Arc::new(DirPool::new(&clone_root_dir)),
                    thread_pool: ThreadPool::new(max_concurrency),
                };
                runner.run();
            })),
        }
    }

    pub fn new_sender(&self) -> Sender<PRMergeMessage> {
        let sender = self.sender.lock().unwrap();
        sender.clone()
    }
}

struct WorkerRunner {
    rx: Receiver<PRMergeMessage>,
    config: Arc<Config>,
    github_session: Arc<Session>,
    dir_pool: Arc<DirPool>,
    thread_pool: ThreadPool,
}

impl WorkerRunner {
    fn run(&self) {
        loop {
            match self.rx.recv() {
                Ok(msg) => {
                    match msg {
                        PRMergeMessage::Stop => break,
                        PRMergeMessage::Merge { repo, pull_request, target_branch } => {
                            self.handle_merge(repo, pull_request, target_branch);
                        }
                    }
                }
                Err(e) => error!("Error receiving message: {}", e),
            };
        }
    }

    fn handle_merge(&self, repo: github::Repo, pull_request: github::PullRequest,
                    target_branch: String) {

        let github_session = self.github_session.clone();
        let dir_pool = self.dir_pool.clone();
        let config = self.config.clone();

        // launch another thread to do the merge
        self.thread_pool.execute(move || {
            if let Err(e) = merge_pull_request(&github_session,
                                               &dir_pool,
                                               &repo.owner.login(),
                                               &repo.name,
                                               &pull_request,
                                               &target_branch) {

                let attach = SlackAttachmentBuilder::new(&e)
                    .title(format!("Source PR: #{}: \"{}\"",
                                   pull_request.number,
                                   pull_request.title)
                        .as_str())
                    .title_link(pull_request.html_url.clone())
                    .color("danger")
                    .build();

                let messenger = messenger::from_config(config);
                messenger.send_to_owner("Error creating merge Pull Request",
                                        &vec![attach],
                                        &pull_request.user,
                                        &repo);
            }
        });
    }
}
