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
    github_session: Arc<github::api::Session>,
}

impl GitCloneManager {
    pub fn new(github_session: Arc<github::api::Session>, config: Arc<Config>) -> GitCloneManager {
        let clone_root_dir = config.main.clone_root_dir.to_string();

        GitCloneManager {
            dir_pool: Arc::new(DirPool::new(&clone_root_dir)),
            github_session: github_session.clone(),
        }
    }

    pub fn clone(&self, owner: &str, repo: &str) -> Result<HeldDir, String> {
        let held_clone_dir = self.dir_pool.take_directory(self.github_session.github_host(), owner, repo);
        self.clone_repo(owner, repo, &held_clone_dir.dir())?;

        Ok(held_clone_dir)
    }

    fn clone_repo(&self, owner: &str, repo: &str, clone_dir: &PathBuf) -> Result<(), String> {
        let url = format!(
            "https://{}@{}/{}/{}",
            self.github_session.user().login(),
            self.github_session.github_host(),
            owner,
            repo
        );

        let git = Git::new(self.github_session.github_host(), self.github_session.github_token(), clone_dir);

        if clone_dir.join(".git").exists() {
            // prune local tags deleted from remotes: important to avoid stale/bad version tags
            git.run(&["fetch", "--prune", "origin", "+refs/tags/*:refs/tags/*"])?;
        } else {
            if let Err(e) = fs::create_dir_all(&clone_dir) {
                return Err(format!("Error creating clone directory '{:?}': {}", clone_dir, e));
            }
            git.run(&["clone", &url, "."])?;
        }

        // always fetch latest tags
        git.run(&["fetch", "--tags"])?;
        // clean up state
        git.clean()?;

        Ok(())
    }
}
