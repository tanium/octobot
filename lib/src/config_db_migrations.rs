use anyhow::anyhow;
use rusqlite::Transaction;
use rusqlite::types::ToSql;

use crate::db;
use crate::db::migrations::{Migration, sql};
use crate::errors::*;

pub fn all_migrations() -> Vec<Box<dyn Migration>> {
    vec![
        sql(r#"
    create table users (
      id integer not null,
      github_name varchar not null,
      slack_name varchar not null,
      UNIQUE( github_name ),
      PRIMARY KEY( id )
    );

    create table repos (
      id integer not null,
      repo varchar not null,
      channel varchar not null,
      force_push_notify tinyint not null,
      force_push_reapply_statuses varchar not null,
      branches varchar not null,
      jira_projects varchar not null,
      jira_versions_enabled tinyint not null,
      version_script varchar not null,
      release_branch_prefix varchar not null,

      UNIQUE( repo, branches ),
      PRIMARY KEY( id )
    );
    "#),
        sql(r#"alter table users add column mute_direct_messages tinyint not null default 0"#),
        sql(r#"alter table repos add column next_branch_suffix varchar not null default ''"#),
        sql(r#"
    create table repos_jiras (
        repo_id integer not null,
        jira varchar not null,
        channel varchar not null,
        version_script varchar not null,
        release_branch_regex varchar not null,

        PRIMARY KEY( repo_id, jira )
    );
    "#),
        Box::new(MigrationReposJiras {}),
        sql(r#"
    create table repos_new (
      id integer not null,
      repo varchar not null,
      channel varchar not null,
      force_push_notify tinyint not null,
      release_branch_prefix varchar not null,

      UNIQUE( repo ),
      PRIMARY KEY( id )
    );

    insert into repos_new
        select id, repo, channel, force_push_notify, release_branch_prefix
        from repos;

    drop table repos;

    alter table repos_new rename to repos;
    "#),
        sql(r#"alter table users add column slack_id varchar not null default ''"#),
        sql(r#"alter table users add column email varchar not null default ''"#),
        sql(r#"alter table repos add column use_threads tinyint not null default 1"#),
        sql(r#"alter table users add column muted_repos varchar not null default ''"#),
        sql(r#"alter table users add column mute_team_dm tinyint not null default 0"#),
    ]
}

struct MigrationReposJiras {}

impl Migration for MigrationReposJiras {
    fn run(&self, tx: &Transaction) -> Result<()> {
        let mut stmt = tx.prepare(r#"SELECT * FROM repos"#)?;
        let cols = db::Columns::from_stmt(&stmt)?;
        let mut rows = stmt.query([])?;

        while let Ok(Some(row)) = rows.next() {
            let id = cols.get::<i32>(row, "id")?;
            let jiras = db::to_string_vec(cols.get(row, "jira_projects")?);
            let version_script = cols.get::<String>(row, "version_script")?;

            for jira in &jiras {
                tx.execute(
                    r#"INSERT INTO repos_jiras (repo_id, jira, version_script, release_branch_regex, channel)
                    VALUES (?1, ?2, ?3, '', '')"#,
                    [
                        &id,
                        jira as &dyn ToSql,
                        &version_script,
                    ],
                )
                .map_err(|e| anyhow!("Error inserting jira repo {} - {}: {}", id, jira, e))?;
            }
        }

        Ok(())
    }
}
