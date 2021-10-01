use failure::format_err;
use rusqlite::types::ToSql;
use rusqlite::Transaction;

use crate::db;
use crate::db::migrations::Migration;
use crate::errors::*;

pub struct MigrationReposJiras {}

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
                    &[
                        &id,
                        jira as &dyn ToSql,
                        &version_script,
                    ],
                )
                .map_err(|e| format_err!("Error inserting jira repo {} - {}: {}", id, jira, e))?;
            }
        }

        Ok(())
    }
}
