use std::time::SystemTime;

use anyhow::anyhow;
use octobot_lib::db::{Connection, Database, migrations};
use octobot_lib::errors::*;
use rusqlite::types::ToSql;

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

        let thread: Option<String> = conn
            .query_row(
                "SELECT thread FROM pull_request_threads WHERE guid = ?1 AND channel = ?2 LIMIT 1",
                [&thread_guid, &slack_channel],
                |row| row.get(0),
            )
            .unwrap_or(None);

        if thread.is_some() {
            conn.execute(
                "UPDATE pull_request_threads SET timestamp = CURRENT_TIMESTAMP WHERE guid = ?1 AND channel = ?2",
                [&thread_guid, &slack_channel],
            )
            .map_err(|e| anyhow!("Error refreshing slack thread {}: {}", thread_guid, e))?;
        }

        Ok(thread)
    }

    pub fn insert_thread(
        &self,
        thread_guid: &str,
        slack_channel: &str,
        thread: &str,
    ) -> Result<()> {
        let conn = self.connect()?;
        conn.execute(
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
        Ok(())
    }

    pub fn clean(&self, expiration: SystemTime) -> Result<()> {
        let deadline = expiration.duration_since(SystemTime::UNIX_EPOCH)?.as_secs();

        let conn = self.connect()?;
        conn.execute(
            "DELETE FROM pull_request_threads WHERE timestamp < datetime(?1, 'unixepoch')",
            [&deadline as &dyn ToSql],
        )
        .map_err(|e| anyhow!("Error cleaning old slack threads: {}", e))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tempfile::{TempDir, tempdir};

    use super::*;

    fn new_test() -> (SlackDatabase, TempDir) {
        let temp_dir = tempdir().unwrap();
        let db_file = temp_dir.path().join("db.sqlite3");
        let db = SlackDatabase::new(&db_file.to_string_lossy()).expect("create temp database");

        (db, temp_dir)
    }

    #[test]
    fn test_lookup_thread() {
        let (db, _temp) = new_test();

        db.insert_thread("org/repo/1", "U123", "1234.5678").unwrap();

        assert_eq!(
            Some("1234.5678".to_string()),
            db.lookup_previous_thread("org/repo/1", "U123").unwrap()
        );
        assert_eq!(
            None,
            db.lookup_previous_thread("org/repo/1", "U456").unwrap()
        );
        assert_eq!(None, db.lookup_previous_thread("", "U123").unwrap());
    }

    #[test]
    fn test_clean() {
        let (db, _temp) = new_test();

        db.insert_thread("org/repo/1", "U123", "1111.1111").unwrap();
        db.connect()
            .unwrap()
            .execute(
                r#"INSERT INTO pull_request_threads (guid, channel, thread, timestamp)
                    VALUES ('org/repo/2', 'U123', '2222.2222', datetime('now', '-31 days'))"#,
                [],
            )
            .unwrap();

        let day = Duration::from_secs(24 * 3600);
        db.clean(SystemTime::now() - 30 * day).unwrap();

        assert!(
            db.lookup_previous_thread("org/repo/1", "U123")
                .unwrap()
                .is_some()
        );
        assert_eq!(
            None,
            db.lookup_previous_thread("org/repo/2", "U123").unwrap()
        );
    }

    #[test]
    fn test_lookup_refreshes_thread() {
        let (db, _temp) = new_test();

        db.connect()
            .unwrap()
            .execute(
                r#"INSERT INTO pull_request_threads (guid, channel, thread, timestamp)
                    VALUES ('org/repo/1', 'U123', '1111.1111', datetime('now', '-31 days'))"#,
                [],
            )
            .unwrap();

        assert!(
            db.lookup_previous_thread("org/repo/1", "U123")
                .unwrap()
                .is_some()
        );

        let day = Duration::from_secs(24 * 3600);
        db.clean(SystemTime::now() - 30 * day).unwrap();

        assert_eq!(
            Some("1111.1111".to_string()),
            db.lookup_previous_thread("org/repo/1", "U123").unwrap()
        );
    }
}
