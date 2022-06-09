use failure::format_err;
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

    pub async fn lookup_previous_thread(
        &self,
        thread_url: String,
        slack_channel: String,
    ) -> Result<Option<String>> {
        let result = self.connect()?;

        let thread = result
            .query_row(
                "SELECT thread FROM pull_request_threads WHERE guid = ?1 AND channel = ?2 LIMIT 1",
                &[&thread_url, &slack_channel],
                |row| row.get(0),
            )
            .map_or_else(|_| None, |r| r);

        Ok(thread)
    }

    pub async fn insert_thread(
        &self,
        thread_guid: &str,
        slack_channel: &str,
        thread: &str,
    ) -> Result<()> {
        let result = self.connect()?;

        result
            .execute(
                r#"INSERT INTO repos_jiras (guid, channel, thread, timestamp)
                    VALUES (?1, ?2, ?3, '?4')"#,
                &[thread_guid, slack_channel, thread, ""],
            )
            .map_err(|e| {
                format_err!(
                    "Error inserting slack thread {} - {} - {}: {}",
                    thread_guid,
                    slack_channel,
                    thread,
                    e
                )
            })?;

        Ok(())
    }
}
