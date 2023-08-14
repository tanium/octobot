use std::sync::Mutex;
use std::time::SystemTime;

use anyhow::anyhow;
use rusqlite::types::ToSql;

use octobot_lib::db::{migrations, Database};
use octobot_lib::errors::*;

use crate::util;
use crate::webhook_db_migrations;

pub struct WebhookDatabase {
    data: Mutex<Data>,
}

struct Data {
    db: Database,
    recent_events: Vec<String>,
}

impl WebhookDatabase {
    pub fn new(db_file: &str) -> Result<WebhookDatabase> {
        let db = Database::new(db_file)?;

        let mut connection = db.connect()?;

        let migrations = webhook_db_migrations::all_migrations();
        migrations::migrate(&mut connection, &migrations)?;

        // Load some recent history into memory at startup
        let mut stmt = connection
            .prepare("SELECT guid FROM processed_webhooks ORDER BY timestamp desc LIMIT 1000")?;
        let found = stmt
            .query_map([], |row| row.get(0))
            .map_err(|e| anyhow!("Error fetching webhooks: {}", e))?;

        let mut recent_events = vec![];
        for event in found {
            recent_events.push(event?);
        }

        Ok(WebhookDatabase {
            data: Mutex::new(Data { db, recent_events }),
        })
    }

    // records the event and returns true if unique, otherwise returns false
    pub fn maybe_record(&self, guid: &str) -> Result<bool> {
        let mut data = self.data.lock().unwrap();

        if self.has_guid(&data, guid) {
            return Ok(false);
        }

        self.record(&mut data, guid)?;
        Ok(true)
    }

    fn record(&self, data: &mut Data, guid: &str) -> Result<()> {
        let timestamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)?
            .as_secs();

        let conn = data.db.connect()?;

        data.recent_events.push(guid.into());

        conn.execute(
            "INSERT INTO processed_webhooks (guid, timestamp) VALUES (?1, ?2)",
            [&guid, &timestamp as &dyn ToSql],
        )
        .map_err(|e| anyhow!("Error inserting webhook {}: {}", guid, e))?;

        Ok(())
    }

    fn has_guid(&self, data: &Data, guid: &str) -> bool {
        // check in-memory cache to avoid hiting db for common case
        if data.recent_events.contains(&guid.to_string()) {
            return true;
        }

        match self.db_has_guid(data, guid) {
            Ok(r) => r,
            Err(e) => {
                log::error!("Error checking guid {}: {}", guid, e);
                false
            }
        }
    }

    fn db_has_guid(&self, data: &Data, guid: &str) -> Result<bool> {
        let conn = data.db.connect()?;
        let mut stmt = conn.prepare("SELECT 1 FROM processed_webhooks where guid = ?1")?;

        stmt.exists([&guid]).map_err(|e| anyhow!("{}", e))
    }

    pub fn clean(&self, expiration: SystemTime) -> Result<()> {
        let deadline = expiration.duration_since(SystemTime::UNIX_EPOCH)?.as_secs();

        let mut data = self.data.lock().unwrap();

        util::trim_unique_events(&mut data.recent_events, 1000, 100);

        let conn = data.db.connect()?;
        conn.execute(
            "DELETE FROM processed_webhooks where timestamp < ?1",
            [&deadline as &dyn ToSql],
        )
        .map_err(|e| anyhow!("Error cleaning webhook db: {}", e))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, time::Duration};

    use super::*;
    use tempfile::{tempdir, TempDir};

    fn new_test() -> (WebhookDatabase, PathBuf, TempDir) {
        let temp_dir = tempdir().unwrap();
        let db_file = temp_dir.path().join("webhook.sqlite3");
        let db = WebhookDatabase::new(&db_file.to_string_lossy()).expect("create temp database");

        (db, db_file, temp_dir)
    }

    fn clean_cache(db: &WebhookDatabase) {
        let mut data = db.data.lock().unwrap();
        data.recent_events.clear();
    }

    #[test]
    fn test_it_works() {
        let (db, db_file, _temp) = new_test();

        let event1 = "event1";
        let event2 = "event2";

        assert!(db.maybe_record(event1).unwrap());
        assert!(db.maybe_record(event2).unwrap());
        assert!(!db.maybe_record(event2).unwrap());
        assert!(!db.maybe_record(event1).unwrap());

        let reload_db =
            WebhookDatabase::new(&db_file.to_string_lossy()).expect("create temp database");
        assert!(!reload_db.maybe_record(event2).unwrap());
        assert!(!reload_db.maybe_record(event1).unwrap());

        clean_cache(&db);
        db.clean(SystemTime::now() - Duration::from_secs(100))
            .unwrap();
        assert!(!db.maybe_record(event1).unwrap());
        assert!(!db.maybe_record(event2).unwrap());

        clean_cache(&db);
        db.clean(SystemTime::now() + Duration::from_secs(100))
            .unwrap();
        assert!(db.maybe_record(event1).unwrap());
        assert!(db.maybe_record(event2).unwrap());
    }
}
