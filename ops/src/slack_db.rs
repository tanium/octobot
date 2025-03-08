use anyhow::anyhow;
use octobot_lib::db::{migrations, Connection, Database};
use octobot_lib::errors::*;

use crate::slack_db_migrations;

#[derive(Clone)]
pub struct SlackDatabase {
    db: Database,
}

impl SlackDatabase {
    pub fn new(db_file: &str) -> Result<SlackDatabase> {
        let db = Database::new(db_file)?;

        let mut connection = db.connect()?;

        let migrations = slack_db_migrations::all_migrations();
        migrations::migrate(&mut connection, &migrations)?;

        Ok(SlackDatabase { db })
    }

    pub fn connect(&self) -> Result<Connection> {
        self.db.connect()
    }

    pub fn lookup_previous_thread(
        &self,
        thread_guid: &str,
        slack_channel: &str,
    ) -> Result<Option<String>> {
        if thread_guid.is_empty() {
            return Ok(None);
        }

        let conn = self.connect()?;

        let thread = conn
            .query_row(
                "SELECT thread FROM pull_request_threads WHERE guid = ?1 AND channel = ?2 LIMIT 1",
                [&thread_guid, &slack_channel],
                |row| row.get(0),
            )
            .unwrap_or(None);

        Ok(thread)
    }

    pub fn insert_thread(
        &self,
        thread_guid: &str,
        slack_channel: &str,
        thread: &str,
    ) -> Result<()> {
        let mut conn = self.connect()?;
        let tx = conn.transaction()?;
        tx.execute(
            r#"INSERT INTO pull_request_threads (guid, channel, thread, timestamp)
                    VALUES (?1, ?2, ?3, CURRENT_TIMESTAMP)"#,
            [thread_guid, slack_channel, thread],
        )
        .map_err(|e| {
            anyhow!(
                "Error inserting slack thread {} - {} - {}: {}",
                thread_guid,
                slack_channel,
                thread,
                e
            )
        })?;
        tx.execute(
            "DELETE FROM pull_request_threads WHERE timestamp < datetime('now', '-1 year')",
            [],
        )
        .map_err(|e| anyhow!("Error cleaning old slack threads: {}", e))?;
        tx.commit()?;
        Ok(())
    }
}
