use rusqlite::Row;

use db::{self, Database};
use errors::*;
use github;

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct RepoInfo {
    pub id: Option<i32>,
    // github org or full repo name. i.e. "some-org" or "some-org/octobot"
    pub repo: String,
    // slack channel to send all messages to
    pub channel: String,
    pub force_push_notify: bool,
    // white-listed statuses to reapply on force-push w/ identical diff
    #[serde(default)]
    pub force_push_reapply_statuses: Vec<String>,
    // list of branches this jira/version config is for
    #[serde(default)]
    pub branches: Vec<String>,
    // A list of jira projects to be respected in processing.
    #[serde(default)]
    pub jira_projects: Vec<String>,
    pub jira_versions_enabled: bool,
    #[serde(default)]
    pub version_script: String,
    // Used for backporting. Defaults to "release/"
    #[serde(default)]
    pub release_branch_prefix: String,
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
            branches: vec![],
            channel: channel.into(),
            force_push_notify: false,
            force_push_reapply_statuses: vec![],
            jira_projects: vec![],
            jira_versions_enabled: false,
            version_script: String::new(),
            release_branch_prefix: String::new(),
        }
    }

    pub fn with_branches(self, value: Vec<String>) -> RepoInfo {
        let mut info = self;
        info.branches = value;
        info
    }

    pub fn with_force_push(self, value: bool) -> RepoInfo {
        let mut info = self;
        info.force_push_notify = value;
        info
    }

    pub fn with_jira(self, value: Vec<String>) -> RepoInfo {
        let mut info = self;
        info.jira_projects = value;
        info
    }

    pub fn with_version_script(self, value: String) -> RepoInfo {
        let mut info = self;
        info.version_script = value;
        info
    }

    pub fn with_release_branch_prefix(self, value: String) -> RepoInfo {
        let mut info = self;
        info.release_branch_prefix = value;
        info
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
        let conn = self.db.connect()?;
        conn.execute(
            r#"INSERT INTO repos (repo, channel, force_push_notify, force_push_reapply_statuses,
                                  branches, jira_projects, jira_versions_enabled, version_script,
                                  release_branch_prefix)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)"#,
            &[
                &repo.repo,
                &repo.channel,
                &db::to_tinyint(repo.force_push_notify),
                &db::from_string_vec(&repo.force_push_reapply_statuses),
                &db::from_string_vec(&repo.branches),
                &db::from_string_vec(&repo.jira_projects),
                &db::to_tinyint(repo.jira_versions_enabled),
                &repo.version_script,
                &repo.release_branch_prefix,
            ],
        ).map_err(|e| Error::from(format!("Error inserting repo {}: {}", repo.repo, e)))?;

        Ok(())
    }

    pub fn update(&mut self, repo: &RepoInfo) -> Result<()> {
        if repo.id.is_none() {
            return Err("Repo does not have an id: cannot update.".into());
        }

        let conn = self.db.connect()?;
        conn.execute(
            r#"UPDATE repos
                SET repo = ?1,
                    channel = ?2,
                    force_push_notify = ?3,
                    force_push_reapply_statuses = ?4,
                    branches = ?5,
                    jira_projects = ?6,
                    jira_versions_enabled = ?7,
                    version_script = ?8,
                    release_branch_prefix = ?9
               WHERE id = ?10"#,
            &[
                &repo.repo,
                &repo.channel,
                &db::to_tinyint(repo.force_push_notify),
                &db::from_string_vec(&repo.force_push_reapply_statuses),
                &db::from_string_vec(&repo.branches),
                &db::from_string_vec(&repo.jira_projects),
                &db::to_tinyint(repo.jira_versions_enabled),
                &repo.version_script,
                &repo.release_branch_prefix,
                &repo.id.unwrap(),
            ],
        ).map_err(|e| Error::from(format!("Error updating repo {}: {}", repo.repo, e)))?;

        Ok(())
    }

    pub fn delete(&mut self, id: i32) -> Result<()> {
        let conn = self.db.connect()?;
        conn.execute("DELETE from repos where id = ?1", &[&id]).map_err(|e| {
            Error::from(format!("Error deleting repo {}: {}", id, e))
        })?;

        Ok(())
    }

    pub fn lookup_channel(&self, repo: &github::Repo) -> Option<String> {
        self.lookup_info(repo, None).map(|r| r.channel.clone())
    }

    pub fn notify_force_push(&self, repo: &github::Repo) -> bool {
        self.lookup_info(repo, None).map(|r| r.force_push_notify).unwrap_or(false)
    }

    pub fn force_push_reapply_statuses(&self, repo: &github::Repo) -> Vec<String> {
        self.lookup_info(repo, None).map(|r| r.force_push_reapply_statuses.clone()).unwrap_or(
            vec![],
        )
    }

    pub fn jira_projects(&self, repo: &github::Repo, branch: &str) -> Vec<String> {
        self.lookup_info(repo, Some(branch)).map(|r| r.jira_projects.clone()).unwrap_or(vec![])
    }

    pub fn jira_versions_enabled(&self, repo: &github::Repo, branch: &str) -> bool {
        self.lookup_info(repo, Some(branch)).map(|r| r.jira_versions_enabled).unwrap_or(false)
    }

    pub fn version_script(&self, repo: &github::Repo, branch: &str) -> Option<String> {
        self.lookup_info(repo, Some(branch))
            .map(|r| if r.version_script.is_empty() {
                None
            } else {
                Some(r.version_script.clone())
            })
            .unwrap_or(None)
    }

    pub fn release_branch_prefix(&self, repo: &github::Repo, branch: &str) -> String {
        let default = "release/".to_string();
        match self.lookup_info(repo, Some(branch)).map(|r| r.release_branch_prefix) {
            None => default,
            Some(ref p) if p.is_empty() => default,
            Some(p) => p,
        }
    }

    pub fn get_all(&self) -> Result<Vec<RepoInfo>> {
        let conn = self.db.connect()?;
        let mut stmt = conn.prepare("SELECT * FROM repos ORDER BY repo")?;
        let cols = db::Columns::from_stmt(&stmt)?;
        let mut rows = stmt.query(&[])?;

        let mut repos = vec![];
        while let Some(row) = rows.next() {
            let row = row?;
            repos.push(self.map_row(&row, &cols)?);
        }

        Ok(repos)
    }

    fn lookup_info(&self, repo: &github::Repo, maybe_branch: Option<&str>) -> Option<RepoInfo> {
        match self.do_lookup_info(repo, maybe_branch) {
            Ok(u) => u,
            Err(e) => {
                error!("Error looking up repo: {}", e);
                None
            }
        }
    }

    fn do_lookup_info(&self, repo: &github::Repo, maybe_branch: Option<&str>) -> Result<Option<RepoInfo>> {
        let conn = self.db.connect()?;
        let mut stmt = conn.prepare(r#"SELECT * FROM repos where repo = :full OR repo = :org"#)?;
        let cols = db::Columns::from_stmt(&stmt)?;
        let mut rows = stmt.query_named(&[(":full", &repo.full_name), (":org", &repo.owner.login())])?;

        let mut repos = Vec::new();
        while let Some(row) = rows.next() {
            let row = row?;
            repos.push(self.map_row(&row, &cols)?);
        }

        // try to match by branch
        if let Some(branch) = maybe_branch {
            for r in &repos {
                if r.repo == repo.full_name && r.branches.contains(&branch.to_string()) {
                    return Ok(Some(r.clone()));
                }
            }
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

    fn map_row(&self, row: &Row, cols: &db::Columns) -> Result<RepoInfo> {
        Ok(RepoInfo {
            id: Some(cols.get(row, "id")?),
            repo: cols.get(row, "repo")?,
            channel: cols.get(row, "channel")?,
            force_push_notify: db::to_bool(cols.get(row, "force_push_notify")?),
            force_push_reapply_statuses: db::to_string_vec(cols.get(row, "force_push_reapply_statuses")?),
            branches: db::to_string_vec(cols.get(row, "branches")?),
            jira_projects: db::to_string_vec(cols.get(row, "jira_projects")?),
            jira_versions_enabled: db::to_bool(cols.get(row, "jira_versions_enabled")?),
            version_script: cols.get(row, "version_script")?,
            release_branch_prefix: cols.get(row, "release_branch_prefix")?,
        })
    }
}

#[cfg(test)]
mod tests {
    extern crate tempdir;

    use self::tempdir::TempDir;
    use super::*;
    use github;

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
        repos.insert_info(&RepoInfo::new("some-user/the-default", "reviews")).unwrap();
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
        repos.insert_info(&RepoInfo::new("some-user/no-config", "reviews")).unwrap();
        repos
            .insert_info(&RepoInfo::new("some-user/with-config", "reviews").with_jira(vec!["a".into(), "b".into()]))
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
    fn test_jira_by_branch() {
        let (mut repos, _temp) = new_test();
        repos.insert("some-user", "SOME_OTHER_CHANNEL").unwrap();

        repos
            .insert_info(&RepoInfo::new("some-user/the-repo", "the-repo-reviews").with_jira(vec!["SOME".into()]))
            .unwrap();

        repos
            .insert_info(&RepoInfo::new("some-user/the-repo", "the-repo-reviews")
                .with_branches(vec!["the-branch".to_string()])
                .with_jira(vec!["THE-BRANCH".into()]))
            .unwrap();

        let repo = github::Repo::parse("http://git.company.com/some-user/the-repo").unwrap();

        assert_eq!(vec!["THE-BRANCH"], repos.jira_projects(&repo, "the-branch"));
        assert_eq!(vec!["SOME"], repos.jira_projects(&repo, "any-other-branch"));
    }
}
