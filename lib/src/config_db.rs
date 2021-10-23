use rusqlite::Connection;

use crate::config_db_migrations;
use crate::db::{self, Database};
use crate::errors::*;

#[derive(Clone)]
pub struct ConfigDatabase {
    db: Database,
}

impl ConfigDatabase {
    pub fn new(db_file: &str) -> Result<ConfigDatabase> {
        let db = Database::new(db_file)?;

        let mut connection = db.connect()?;
        migrate(&mut connection)?;

        Ok(ConfigDatabase { db })
    }

    pub fn connect(&self) -> Result<Connection> {
        self.db.connect()
    }
}

fn migrate(conn: &mut Connection) -> Result<()> {
    let migrations = config_db_migrations::all_migrations();
    db::migrations::migrate(conn, &migrations)
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

        assert_eq!(
            Some((config_db_migrations::all_migrations().len() as i32) - 1),
            db::migrations::current_version(&conn).unwrap()
        );
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
