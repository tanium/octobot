use rusqlite::Connection;

use errors::*;

const CREATE_VERSIONS: &'static str = r#"
create table __version ( current_version integer primary key )
"#;

const MIGRATIONS: [&'static str; 2] = [
    r#"
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
    "#,
    r#"alter table users add column mute_direct_messages tinyint not null default 0"#,
];

fn current_version(conn: &Connection) -> Result<Option<i32>> {
    let mut version: Option<i32> = None;
    conn.query_row("SELECT current_version from __version", rusqlite::NO_PARAMS, |row| {
        version = row.get(0).ok();
        Ok(())
    })
    .map_err(|_| Error::from("Could not get current version"))?;

    Ok(version)
}

pub fn migrate(conn: &mut Connection) -> Result<()> {
    let version: Option<i32> = match current_version(conn) {
        Ok(v) => v,
        Err(_) => {
            // versions table probably doesn't exist.
            conn.execute(CREATE_VERSIONS, rusqlite::NO_PARAMS)
                .map_err(|e| Error::from(format!("Error creating versions table: {}", e)))?;
            None
        }
    };

    info!("Current schema version: {:?}", version);

    let mut next_version = version.map(|v| v + 1).unwrap_or(0);
    while next_version < MIGRATIONS.len() as i32 {
        info!("Migrating to schema version: {:}", next_version);
        let tx = conn.transaction()?;

        let next_version_unsigned: usize = next_version as usize;

        tx.execute_batch(MIGRATIONS[next_version_unsigned])
            .map_err(|e| Error::from(format!("Error running migrations: {}", e)))?;

        if next_version == 0 {
            tx.execute("INSERT INTO __version VALUES (?1)", &[&next_version])
        } else {
            tx.execute("UPDATE __version set current_version = ?1", &[&next_version])
        }
        .map_err(|e| Error::from(format!("Error updating version: {}", e)))?;

        tx.commit()?;

        next_version += 1;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    extern crate tempdir;

    use self::tempdir::TempDir;
    use super::*;

    #[test]
    fn test_migration_versions() {
        let temp_dir = TempDir::new("users.rs").unwrap();
        let db_file = temp_dir.path().join("db.sqlite3");
        let mut conn = Connection::open(&db_file).expect("create temp database");

        migrate(&mut conn).unwrap();

        assert_eq!(Some((MIGRATIONS.len() as i32) - 1), current_version(&conn).unwrap());
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
