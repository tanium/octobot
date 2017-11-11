use rusqlite::Connection;

use errors::*;

#[derive(Clone)]
pub struct Database {
    db_file: String,
}

impl Database {
    pub fn new(db_file: &str) -> Result<Database> {
        let mut db = Database { db_file: db_file.to_string() };

        db.migrate()?;
        Ok(db)
    }

    pub fn connect(&self) -> Result<Connection> {
        Connection::open(&self.db_file).map_err(|e| {
            Error::from(format!("Error opening database {}: {}", self.db_file, e))
        })
    }

    fn migrate(&mut self) -> Result<()> {
        let sql = include_str!("db.sql");
        let conn = self.connect()?;
        conn.query_row("PRAGMA journal_mode=WAL", &[], |row| {
            let res: String = row.get(0);
            if res.trim() != "wal" {
                error!("Error setting WAL mode. Result: {}", res);
            }
        }).map_err(|e| {
            Error::from(format!("Error turning on WAL mode: {}", e))
        })?;

        conn.execute_batch(sql).map_err(
            |e| Error::from(format!("Error running migrations: {}", e)),
        )?;

        Ok(())
    }
}
