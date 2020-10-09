use failure::format_err;
use log::info;
use rusqlite::{Connection, Transaction};

use crate::db::migrations_code;
use crate::errors::*;

const CREATE_VERSIONS: &'static str = r#"
create table __version ( current_version integer primary key )
"#;

pub trait Migration {
    fn run(&self, tx: &Transaction) -> Result<()>;
}

struct SQLMigration {
    sql: &'static str,
}

impl Migration for SQLMigration {
    fn run(&self, tx: &Transaction) -> Result<()> {
        tx.execute_batch(self.sql)
            .map_err(|e| format_err!("Error running migration: \n---\n{}\n---\n. Error: {}", self.sql, e))
    }
}

fn sql(s: &'static str) -> Box<dyn Migration> {
    Box::new(SQLMigration { sql: s })
}

fn all_migrations() -> Vec<Box<dyn Migration>> {
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
        Box::new(migrations_code::MigrationReposJiras {}),
        sql(r#"
    create table repos_new (
      id integer not null,
      repo varchar not null,
      channel varchar not null,
      force_push_notify tinyint not null,
      force_push_reapply_statuses varchar not null,
      branches varchar not null,
      release_branch_prefix varchar not null,
      next_branch_suffix varchar not null default '',

      UNIQUE( repo, branches ),
      PRIMARY KEY( id )
    );

    insert into repos_new
        select id, repo, channel, force_push_notify, force_push_reapply_statuses, branches, release_branch_prefix, next_branch_suffix
        from repos;

    drop table repos;

    alter table repos_new rename to repos;
    "#),
    ]
}

fn current_version(conn: &Connection) -> Result<Option<i32>> {
    let mut version: Option<i32> = None;
    conn.query_row("SELECT current_version from __version", rusqlite::NO_PARAMS, |row| {
        version = row.get(0).ok();
        Ok(())
    })
    .map_err(|_| format_err!("Could not get current version"))?;

    Ok(version)
}

pub fn migrate(conn: &mut Connection) -> Result<()> {
    let version: Option<i32> = match current_version(conn) {
        Ok(v) => v,
        Err(_) => {
            // versions table probably doesn't exist.
            conn.execute(CREATE_VERSIONS, rusqlite::NO_PARAMS)
                .map_err(|e| format_err!("Error creating versions table: {}", e))?;
            None
        }
    };

    info!("Current schema version: {:?}", version);

    let migrations = all_migrations();

    let mut next_version = version.map(|v| v + 1).unwrap_or(0);
    while next_version < migrations.len() as i32 {
        info!("Migrating to schema version: {:}", next_version);
        let tx = conn.transaction()?;

        let next_version_unsigned: usize = next_version as usize;

        migrations[next_version_unsigned].run(&tx)?;

        if next_version == 0 {
            tx.execute("INSERT INTO __version VALUES (?1)", &[&next_version])
        } else {
            tx.execute("UPDATE __version set current_version = ?1", &[&next_version])
        }
        .map_err(|e| format_err!("Error updating version: {}", e))?;

        tx.commit()?;

        next_version += 1;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempdir::TempDir;

    #[test]
    fn test_migration_versions() {
        let temp_dir = TempDir::new("users.rs").unwrap();
        let db_file = temp_dir.path().join("db.sqlite3");
        let mut conn = Connection::open(&db_file).expect("create temp database");

        migrate(&mut conn).unwrap();

        assert_eq!(Some((all_migrations().len() as i32) - 1), current_version(&conn).unwrap());
    }

    #[test]
    fn test_multiple_migration() {
        let temp_dir = TempDir::new("users.rs").unwrap();
        let db_file = temp_dir.path().join("db.sqlite3");
        let mut conn = Connection::open(&db_file).expect("create temp database");

        // migration #1
        migrate(&mut conn).unwrap();

        // migration #2
        if let Err(e) = migrate(&mut conn) {
            panic!("Failed: expected second migration to be a noop: {}", e);
        }
    }
}
