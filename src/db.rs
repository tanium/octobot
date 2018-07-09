use std::collections::HashMap;

use rusqlite::{Connection, Row, Statement};
use rusqlite::types::FromSql;

use db_migrations;
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
        let mut conn = self.connect()?;
        conn.query_row("PRAGMA journal_mode=WAL", &[], |row| {
            let res: String = row.get(0);
            if res.trim() != "wal" {
                error!("Error setting WAL mode. Result: {}", res);
            }
        }).map_err(|e| Error::from(format!("Error turning on WAL mode: {}", e)))?;


        db_migrations::migrate(&mut conn)
    }
}

pub struct Columns {
    cols: HashMap<String, i32>,
}

impl Columns {
    pub fn from_stmt(stmt: &Statement) -> Result<Columns> {
        let names = stmt.column_names();
        let mut cols = HashMap::new();

        for name in names {
            let index = stmt.column_index(name)?;
            cols.insert(name.to_string(), index);
        }

        Ok(Columns { cols: cols })
    }

    pub fn get_index(&self, col: &str) -> Result<i32> {
        self.cols.get(col).map(|i| i.clone()).ok_or(
            Error::from(format!("Invalid columnL '{}'", col)),
        )
    }

    pub fn get<T: FromSql>(&self, row: &Row, col: &str) -> Result<T> {
        let index = self.get_index(col)?;
        row.get_checked(index).map_err(
            |e| Error::from(format!("Error getting column {}: {}", col, e)),
        )
    }
}


pub fn from_string_vec(val: &Vec<String>) -> String {
    val.join(",")
}

pub fn to_string_vec(val: String) -> Vec<String> {
    if val.is_empty() {
        vec![]
    } else {
        val.split(",").map(|s| s.trim().to_string()).collect::<Vec<_>>()
    }
}

pub fn to_bool(val: i32) -> bool {
    val != 0
}

pub fn to_tinyint(val: bool) -> i8 {
    if val { 1 } else { 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bools() {
        assert_eq!(1, to_tinyint(true));
        assert_eq!(0, to_tinyint(false));
        assert_eq!(true, to_bool(1));
        assert_eq!(false, to_bool(0));
    }

    #[test]
    fn test_vecs() {
        assert_eq!("hello,world", from_string_vec(&vec!["hello".into(), "world".into()]));
        assert_eq!(vec!["hello".to_string(), "world".to_string()], to_string_vec("hello,world".into()));
        assert_eq!(vec!["hello".to_string(), "world".to_string()], to_string_vec("  hello ,  world  ".into()));
    }

    #[test]
    fn test_vecs_empty() {
        assert_eq!("", from_string_vec(&Vec::<String>::new()));
        assert_eq!(Vec::<String>::new(), to_string_vec(String::new()));
    }
}
