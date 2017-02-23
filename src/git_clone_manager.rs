use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use config::Config;
use dir_pool::{DirPool, HeldDir};
use git::Git;
use github;

// clones git repos with given github session into a managed directory pool
pub struct GitCloneManager {
    dir_pool: Arc<DirPool>,
    git: Git,
    github_session: Arc<github::api::Session>,
}

impl GitCloneManager {
    pub fn new(github_session: Arc<github::api::Session>, config: Arc<Config>) -> GitCloneManager {
        let clone_root_dir = config.clone_root_dir.to_string();

        GitCloneManager {
            dir_pool: Arc::new(DirPool::new(&clone_root_dir)),
            git: Git::new(github_session.github_host(), github_session.github_token()),
            github_session: github_session.clone(),
        }
    }

    pub fn clone(&self, owner: &str, repo: &str) -> Result<HeldDir, String> {
        let held_clone_dir = self.dir_pool.take_directory(self.github_session.github_host(), owner, repo);
        try!(self.clone_repo(owner, repo, &held_clone_dir.dir()));

        Ok(held_clone_dir)
    }

    fn clone_repo(&self, owner: &str, repo: &str, clone_dir: &PathBuf) -> Result<(), String> {
        let url = format!("https://{}@{}/{}/{}",
                          self.github_session.user().login(),
                          self.github_session.github_host(),
                          owner,
                          repo);

        if clone_dir.join(".git").exists() {
            try!(self.git.run(&["fetch", "--prune"], clone_dir));
        } else {
            if let Err(e) = fs::create_dir_all(&clone_dir) {
                return Err(format!("Error creating clone directory '{:?}': {}", clone_dir, e));
            }
            try!(self.git.run(&["clone", &url, "."], clone_dir));
        }

        // always fetch latest tags
        try!(self.git.run(&["fetch", "--tags"], clone_dir));

        Ok(())
    }
}
