use failure::format_err;
use log::error;
use rusqlite::types::ToSql;
use rusqlite::{Connection, Row, Transaction};
use serde_derive::{Deserialize, Serialize};

use crate::db::{self, Database};
use crate::errors::*;
use crate::github;

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct RepoInfo {
    pub id: Option<i32>,
    // github org or full repo name. i.e. "some-org" or "some-org/octobot"
    pub repo: String,
    // slack channel to send all messages to
    pub channel: String,
    pub force_push_notify: bool,
    // A list of jira projects to be respected in processing.
    #[serde(default)]
    pub jira_config: Vec<RepoJiraConfig>,
    // Used for backporting. Defaults to "release/"
    #[serde(default)]
    pub release_branch_prefix: String,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct RepoJiraConfig {
    // The jira project key
    #[serde(default)]
    pub jira_project: String,

    // The version script to use for this JIRA project
    #[serde(default)]
    pub version_script: String,

    // A regex that matches release branchs that are relevant to this JIRA project.
    // If left blank, it matches all release branches.
    #[serde(default)]
    pub release_branch_regex: String,
}

#[derive(Clone)]
pub struct RepoConfig {
    db: Database,
}

impl RepoInfo {
    pub fn new(repo: &str, channel: &str) -> RepoInfo {
        RepoInfo {
            id: None,
            repo: repo.into(),
            channel: channel.into(),
            force_push_notify: false,
            jira_config: vec![],
            release_branch_prefix: String::new(),
        }
    }

    pub fn with_force_push(self, value: bool) -> RepoInfo {
        let mut info = self;
        info.force_push_notify = value;
        info
    }

    pub fn with_jira(self, jira_project: &str) -> RepoInfo {
        self.with_jira_config(RepoJiraConfig::new(jira_project))
    }

    pub fn with_jira_config(self, config: RepoJiraConfig) -> RepoInfo {
        let mut info = self;

        info.jira_config.push(config);
        info
    }

    pub fn with_release_branch_prefix(self, value: String) -> RepoInfo {
        let mut info = self;
        info.release_branch_prefix = value;
        info
    }
}

impl RepoJiraConfig {
    pub fn new(jira_project: &str) -> RepoJiraConfig {
        RepoJiraConfig {
            jira_project: jira_project.into(),
            version_script: String::new(),
            release_branch_regex: String::new(),
        }
    }

    pub fn with_version_script(self, value: &str) -> RepoJiraConfig {
        let mut c = self;
        c.version_script = value.into();
        c
    }

    pub fn with_release_branch_regex(self, value: &str) -> RepoJiraConfig {
        let mut c = self;
        c.release_branch_regex = value.into();
        c
    }
}

impl RepoConfig {
    pub fn new(db: Database) -> RepoConfig {
        RepoConfig { db: db }
    }

    pub fn insert(&mut self, repo: &str, channel: &str) -> Result<()> {
        self.insert_info(&RepoInfo::new(repo, channel))
    }

    pub fn insert_info(&mut self, repo: &RepoInfo) -> Result<()> {
        let mut conn = self.db.connect()?;
        let tx = conn.transaction()?;

        tx.execute(
            r#"INSERT INTO repos (repo, channel, force_push_notify, release_branch_prefix)
               VALUES (?1, ?2, ?3, ?4)"#,
            &[
                &repo.repo,
                &repo.channel,
                &db::to_tinyint(repo.force_push_notify) as &dyn ToSql,
                &repo.release_branch_prefix,
            ],
        )
        .map_err(|e| format_err!("Error inserting repo {}: {}", repo.repo, e))?;

        let id = tx.last_insert_rowid();
        self.insert_jiras(&tx, id, &repo.jira_config)?;

        tx.commit()?;

        Ok(())
    }

    pub fn update(&mut self, repo: &RepoInfo) -> Result<()> {
        if repo.id.is_none() {
            return Err(format_err!("Repo does not have an id: cannot update."));
        }
        let id = repo.id.unwrap();

        let mut conn = self.db.connect()?;
        let tx = conn.transaction()?;

        tx.execute(
            r#"UPDATE repos
                SET repo = ?1,
                    channel = ?2,
                    force_push_notify = ?3,
                    release_branch_prefix = ?4
               WHERE id = ?5"#,
            &[
                &repo.repo,
                &repo.channel,
                &db::to_tinyint(repo.force_push_notify) as &dyn ToSql,
                &repo.release_branch_prefix,
                &id,
            ],
        )
        .map_err(|e| format_err!("Error updating repo {}: {}", repo.repo, e))?;

        tx.execute(r#"DELETE from repos_jiras where repo_id = ?1"#, &[&id])
            .map_err(|e| format_err!("Error clearing repo jira entries {}: {}", repo.repo, e))?;

        self.insert_jiras(&tx, id as i64, &repo.jira_config)?;

        tx.commit()?;

        Ok(())
    }

    fn insert_jiras(&mut self, tx: &Transaction, id: i64, jira_config: &Vec<RepoJiraConfig>) -> Result<()> {
        for config in jira_config {
            tx.execute(
                r#"INSERT INTO repos_jiras (repo_id, jira, version_script, release_branch_regex)
               VALUES (?1, ?2, ?3, ?4)"#,
                &[
                    &id,
                    &config.jira_project as &dyn ToSql,
                    &config.version_script,
                    &config.release_branch_regex,
                ],
            )
            .map_err(|e| format_err!("Error inserting jira {} for repo {}: {}", config.jira_project, id, e))?;
        }

        Ok(())
    }

    pub fn delete(&mut self, id: i32) -> Result<()> {
        let conn = self.db.connect()?;
        conn.execute("DELETE from repos where id = ?1", &[&id])
            .map_err(|e| format_err!("Error deleting repo {}: {}", id, e))?;

        Ok(())
    }

    pub fn lookup_channel(&self, repo: &github::Repo) -> Option<String> {
        self.lookup_info(repo).map(|r| r.channel.clone())
    }

    pub fn notify_force_push(&self, repo: &github::Repo) -> bool {
        self.lookup_info(repo)
            .map(|r| r.force_push_notify)
            .unwrap_or(false)
    }

    pub fn jira_configs(&self, repo: &github::Repo, branch: &str) -> Vec<RepoJiraConfig> {
        let mut configs = self
            .lookup_info(repo)
            .map(|r| r.jira_config.clone())
            .unwrap_or(vec![]);

        configs.retain(|c| github::is_main_branch(branch) || self.matches_branch(branch, &c.release_branch_regex));

        configs
    }

    fn matches_branch(&self, branch: &str, regex: &str) -> bool {
        match regex::Regex::new(regex) {
            Ok(r) => r.is_match(branch),
            Err(e) => {
                log::error!("Error parsing branch regex: '{}': {}", regex, e);
                false
            }
        }
    }

    pub fn jira_projects(&self, repo: &github::Repo, branch: &str) -> Vec<String> {
        self.jira_configs(repo, branch)
            .into_iter()
            .map(|c| c.jira_project)
            .collect::<Vec<_>>()
    }

    pub fn release_branch_prefix(&self, repo: &github::Repo) -> String {
        let default = "release/".to_string();
        match self.lookup_info(repo).map(|r| r.release_branch_prefix) {
            None => default,
            Some(ref p) if p.is_empty() => default,
            Some(p) => p,
        }
    }

   pub fn get_all(&self) -> Result<Vec<RepoInfo>> {
        let conn = self.db.connect()?;
        let mut stmt = conn.prepare("SELECT * FROM repos ORDER BY repo")?;
        let cols = db::Columns::from_stmt(&stmt)?;
        let mut rows = stmt.query(rusqlite::NO_PARAMS)?;

        let mut repos = vec![];
        while let Ok(Some(row)) = rows.next() {
            repos.push(self.map_row(&conn, &row, &cols)?);
        }

        Ok(repos)
    }

    fn lookup_info(&self, repo: &github::Repo) -> Option<RepoInfo> {
        match self.do_lookup_info(repo) {
            Ok(u) => u,
            Err(e) => {
                error!("Error looking up repo: {}", e);
                None
            }
        }
    }

    fn do_lookup_info(&self, repo: &github::Repo) -> Result<Option<RepoInfo>> {
        let conn = self.db.connect()?;
        let mut stmt = conn.prepare(r#"SELECT * FROM repos where repo = :full OR repo = :org"#)?;
        let cols = db::Columns::from_stmt(&stmt)?;
        let mut rows = stmt.query_named(&[(":full", &repo.full_name), (":org", &repo.owner.login())])?;

        let mut repos = Vec::new();
        while let Ok(Some(row)) = rows.next() {
            repos.push(self.map_row(&conn, &row, &cols)?);
        }

        // try to match by org/repo
        for r in &repos {
            if r.repo == repo.full_name {
                return Ok(Some(r.clone()));
            }
        }
        // try to match by org
        for r in &repos {
            if r.repo == repo.owner.login() {
                return Ok(Some(r.clone()));
            }
        }

        Ok(None)
    }

    fn map_row(&self, conn: &Connection, row: &Row, cols: &db::Columns) -> Result<RepoInfo> {
        let id = cols.get(row, "id")?;
        let jira_config = self.load_jira_config(&conn, id)?;

        Ok(RepoInfo {
            id: Some(id),
            repo: cols.get(row, "repo")?,
            channel: cols.get(row, "channel")?,
            force_push_notify: db::to_bool(cols.get(row, "force_push_notify")?),
            jira_config: jira_config,
            release_branch_prefix: cols.get(row, "release_branch_prefix")?,
        })
    }

    fn load_jira_config(&self, conn: &Connection, id: i32) -> Result<Vec<RepoJiraConfig>> {
        let mut stmt = conn.prepare(r#"SELECT * FROM repos_jiras where repo_id = :id"#)?;
        let cols = db::Columns::from_stmt(&stmt)?;
        let mut rows = stmt.query_named(&[(":id", &id)])?;

        let mut result = vec![];
        while let Ok(Some(row)) = rows.next() {
            let config = RepoJiraConfig {
                jira_project: cols.get(row, "jira")?,
                version_script: cols.get(row, "version_script")?,
                release_branch_regex: cols.get(row, "release_branch_regex")?,
            };

            result.push(config);
        }

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use github;
    use tempdir::TempDir;

    fn new_test() -> (RepoConfig, TempDir) {
        let temp_dir = TempDir::new("repos.rs").unwrap();
        let db_file = temp_dir.path().join("db.sqlite3");
        let db = Database::new(&db_file.to_string_lossy()).expect("create temp database");

        (RepoConfig::new(db), temp_dir)
    }

    #[test]
    fn lookup_channel_by_repo_full_name() {
        let (mut repos, _temp) = new_test();
        // insert org-level one first in the list to make sure most specific matches first
        repos.insert("some-user", "SOME_OTHER_CHANNEL").unwrap();
        repos.insert("some-user/the-repo", "the-repo-reviews").unwrap();

        let repo = github::Repo::parse("http://git.company.com/some-user/the-repo").unwrap();
        assert_eq!("the-repo-reviews", repos.lookup_channel(&repo).unwrap());
    }

    #[test]
    fn lookup_channel_by_repo_owner() {
        let (mut repos, _temp) = new_test();
        repos.insert("some-user", "the-repo-reviews").unwrap();

        let repo = github::Repo::parse("http://git.company.com/some-user/some-other-repo").unwrap();
        assert_eq!("the-repo-reviews", repos.lookup_channel(&repo).unwrap());
    }

    #[test]
    fn lookup_channel_none() {
        let (mut repos, _temp) = new_test();
        repos.insert("some-user", "the-repo-reviews").unwrap();

        // fail by channel/repo
        {
            let repo = github::Repo::parse("http://git.company.com/someone-else/some-other-repo").unwrap();
            assert!(repos.lookup_channel(&repo).is_none());
        }
    }

    #[test]
    fn test_notify_force_push() {
        let (mut repos, _temp) = new_test();
        repos
            .insert_info(&RepoInfo::new("some-user/the-default", "reviews"))
            .unwrap();
        repos
            .insert_info(&RepoInfo::new("some-user/on-purpose", "reviews").with_force_push(true))
            .unwrap();
        repos
            .insert_info(&RepoInfo::new("some-user/quiet-repo", "reviews").with_force_push(false))
            .unwrap();
        {
            let repo = github::Repo::parse("http://git.company.com/someone-else/some-other-repo").unwrap();
            assert_eq!(false, repos.notify_force_push(&repo));
        }

        {
            let repo = github::Repo::parse("http://git.company.com/some-user/the-default").unwrap();
            assert_eq!(false, repos.notify_force_push(&repo));
        }

        {
            let repo = github::Repo::parse("http://git.company.com/some-user/on-purpose").unwrap();
            assert_eq!(true, repos.notify_force_push(&repo));
        }

        {
            let repo = github::Repo::parse("http://git.company.com/some-user/quiet-repo").unwrap();
            assert_eq!(false, repos.notify_force_push(&repo));
        }
    }

    #[test]
    fn test_jira_enabled() {
        let (mut repos, _temp) = new_test();
        repos
            .insert_info(&RepoInfo::new("some-user/no-config", "reviews"))
            .unwrap();
        repos
            .insert_info(
                &RepoInfo::new("some-user/with-config", "reviews")
                    .with_jira("a")
                    .with_jira("b"),
            )
            .unwrap();

        {
            let repo = github::Repo::parse("http://git.company.com/someone-else/some-other-repo").unwrap();
            assert_eq!(Vec::<String>::new(), repos.jira_projects(&repo, "any"));
        }

        {
            let repo = github::Repo::parse("http://git.company.com/some-user/no-config").unwrap();
            assert_eq!(Vec::<String>::new(), repos.jira_projects(&repo, "any"));
        }

        {
            let repo = github::Repo::parse("http://git.company.com/some-user/with-config").unwrap();
            assert_eq!(vec!["a", "b"], repos.jira_projects(&repo, "any"));
        }
    }

    #[test]
    fn test_jira_repos_config() {
        let (mut repos, _temp) = new_test();
        repos.insert("some-user", "SOME_OTHER_CHANNEL").unwrap();

        let config1 = RepoJiraConfig::new("SER")
            .with_release_branch_regex("release/server-.*");
        let config2 = RepoJiraConfig::new("CLI")
            .with_release_branch_regex("release/client-.*");

        repos
            .insert_info(
                &RepoInfo::new("some-user/the-repo", "the-repo-reviews")
                    .with_jira_config(config1)
                    .with_jira_config(config2),
            )
            .unwrap();

        let repo = github::Repo::parse("http://git.company.com/some-user/the-repo").unwrap();

        assert_eq!(vec!["CLI", "SER"], repos.jira_projects(&repo, "master"));
        assert_eq!(vec!["CLI", "SER"], repos.jira_projects(&repo, "main"));
        assert_eq!(vec!["CLI", "SER"], repos.jira_projects(&repo, "develop"));

        assert_eq!(vec!["SER"], repos.jira_projects(&repo, "release/server-1.2"));
        assert_eq!(vec!["CLI"], repos.jira_projects(&repo, "release/client-5.6"));
        assert_eq!(Vec::<String>::new(), repos.jira_projects(&repo, "release/other"));
    }

    #[test]
    fn test_repos_update() {
        let (mut repos, _temp) = new_test();
        repos.insert("some-user", "SOME_OTHER_CHANNEL").unwrap();

        let mut all = repos.get_all().unwrap();
        assert_eq!(1, all.len());

        all[0].channel = "new-channel".into();
        repos.update(&all[0]).unwrap();

        let all = repos.get_all().unwrap();
        assert_eq!(1, all.len());
        assert_eq!("new-channel", all[0].channel);
    }
}
