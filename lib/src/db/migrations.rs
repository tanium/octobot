use anyhow::anyhow;
use log::info;
use rusqlite::{Connection, Transaction};

use crate::errors::*;

const CREATE_VERSIONS: &str = r#"
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
        tx.execute_batch(self.sql).map_err(|e| {
            anyhow!(
                "Error running migration: \n---\n{}\n---\n. Error: {}",
                self.sql,
                e
            )
        })
    }
}

pub fn sql(s: &'static str) -> Box<dyn Migration> {
    Box::new(SQLMigration { sql: s })
}

pub fn current_version(conn: &Connection) -> Result<Option<i32>> {
    let mut version: Option<i32> = None;
    conn.query_row("SELECT current_version from __version", [], |row| {
        version = row.get(0).ok();
        Ok(())
    })
    .map_err(|_| anyhow!("Could not get current version"))?;

    Ok(version)
}

pub fn migrate(conn: &mut Connection, migrations: &[Box<dyn Migration>]) -> Result<()> {
    let version: Option<i32> = match current_version(conn) {
        Ok(v) => v,
        Err(_) => {
            // versions table probably doesn't exist.
            conn.execute(CREATE_VERSIONS, [])
                .map_err(|e| anyhow!("Error creating versions table: {}", e))?;
            None
        }
    };

    info!("Current schema version: {:?}", version);

    let mut next_version = version.map(|v| v + 1).unwrap_or(0);
    while next_version < migrations.len() as i32 {
        info!("Migrating to schema version: {:}", next_version);
        let tx = conn.transaction()?;

        let next_version_unsigned: usize = next_version as usize;

        migrations[next_version_unsigned].run(&tx)?;

        if next_version == 0 {
            tx.execute("INSERT INTO __version VALUES (?1)", [&next_version])
        } else {
            tx.execute("UPDATE __version set current_version = ?1", [&next_version])
        }
        .map_err(|e| anyhow!("Error updating version: {}", e))?;

        tx.commit()?;

        next_version += 1;
    }

    Ok(())
}
