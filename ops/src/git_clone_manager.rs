use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use log::info;

use crate::dir_pool::{ArcDirPool, HeldDir};
use crate::git::Git;
use octobot_lib::config::Config;
use octobot_lib::errors::*;
use octobot_lib::github;
use octobot_lib::github::api::Session;

// clones git repos with given github session into a managed directory pool
pub struct GitCloneManager {
    dir_pool: ArcDirPool,
    github_app: Arc<dyn github::api::GithubSessionFactory>,
}

impl GitCloneManager {
    pub fn new(
        github_app: Arc<dyn github::api::GithubSessionFactory>,
        config: Arc<Config>,
    ) -> GitCloneManager {
        let clone_root_dir = config.main.clone_root_dir.to_string();

        GitCloneManager {
            dir_pool: ArcDirPool::new(&clone_root_dir),
            github_app: github_app.clone(),
        }
    }

    pub async fn clone(&self, owner: &str, repo: &str) -> Result<HeldDir> {
        let session = self.github_app.new_session(owner, repo).await?;

        let held_clone_dir = self
            .dir_pool
            .take_directory(session.github_host(), owner, repo);
        self.clone_repo(&session, owner, repo, held_clone_dir.dir())?;

        Ok(held_clone_dir)
    }

    pub fn clean(&self, expiration: Duration) {
        self.dir_pool.clean(expiration);
    }

    fn clone_repo(
        &self,
        session: &dyn github::api::Session,
        owner: &str,
        repo: &str,
        clone_dir: &Path,
    ) -> Result<()> {
        let url = format!(
            "https://x-access-token@{}/{}/{}",
            session.github_host(),
            owner,
            repo
        );

        let git = Git::new(session.github_host(), session.github_token(), clone_dir);

        if clone_dir.join(".git").exists() {
            info!(
                "Reusing cloned repo https://{}/{}/{} in {:?}",
                session.github_host(),
                owner,
                repo,
                clone_dir
            );
            // prune local tags deleted from remotes: important to avoid stale/bad version tags
            git.run(&["fetch", "--prune", "origin", "+refs/tags/*:refs/tags/*"])?;
        } else {
            info!(
                "Cloning https://{}/{}/{} into {:?}",
                session.github_host(),
                owner,
                repo,
                clone_dir
            );
            if let Err(e) = fs::create_dir_all(clone_dir) {
                return Err(anyhow!(
                    "Error creating clone directory '{:?}': {}",
                    clone_dir,
                    e
                ));
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
