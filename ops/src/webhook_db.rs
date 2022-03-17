use octobot_lib::db::{migrations, Connection, Database};
use octobot_lib::errors::*;

use crate::webhook_db_migrations;

#[derive(Clone)]
pub struct WebhookDatabase {
    db: Database,
}

impl WebhookDatabase {
    pub fn new(db_file: &str) -> Result<WebhookDatabase> {
        let db = Database::new(db_file)?;

        let mut connection = db.connect()?;

        let migrations = webhook_db_migrations::all_migrations();
        migrations::migrate(&mut connection, &migrations)?;

        Ok(WebhookDatabase { db })
    }

    pub fn connect(&self) -> Result<Connection> {
        self.db.connect()
    }
}
