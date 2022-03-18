use failure::format_err;
use log::error;
use rusqlite::types::ToSql;
use serde_derive::{Deserialize, Serialize};

use crate::config_db::ConfigDatabase;
use crate::db;
use crate::errors::*;
use crate::slack::SlackChannel;

#[derive(Deserialize, Serialize, Clone)]
pub struct UserInfo {
    pub id: Option<i32>,
    pub github: String,
    pub slack_name: String,
    pub slack_id: String,
    pub email: String,
    pub mute_direct_messages: bool,
}

#[derive(Clone)]
pub struct UserConfig {
    db: ConfigDatabase,
}

impl UserInfo {
    pub fn new(git_user: &str, slack_user: &str, slack_id: &str, email: &str) -> UserInfo {
        UserInfo {
            id: None,
            github: git_user.to_string(),
            slack_name: slack_user.to_string(),
            slack_id: slack_id.to_string(),
            email: email.to_string(),
            mute_direct_messages: false,
        }
    }
}

impl UserConfig {
    pub fn new(db: ConfigDatabase) -> UserConfig {
        UserConfig { db }
    }

    pub fn insert(&mut self, git_user: &str, slack_user: &str) -> Result<()> {
        self.insert_info(&UserInfo::new(git_user, slack_user, "", ""))
    }

    pub fn insert_info(&mut self, user: &UserInfo) -> Result<()> {
        let conn = self.db.connect()?;
        conn.execute(
            "INSERT INTO users (github_name, slack_name, slack_id, email, mute_direct_messages) VALUES (?1, ?2, ?3, ?4, ?5)",
            &[
                &user.github,
                &user.slack_name,
                &user.slack_id,
                &user.email,
                &db::to_tinyint(user.mute_direct_messages) as &dyn ToSql,
            ],
        )
        .map_err(|e| format_err!("Error inserting user {}: {}", user.github, e))?;

        Ok(())
    }

    pub fn update(&mut self, user: &UserInfo) -> Result<()> {
        let conn = self.db.connect()?;
        conn.execute(
            "UPDATE users set github_name = ?1, slack_name = ?2, slack_id = ?3, email = ?4, mute_direct_messages = ?5 where id = ?6",
            &[&user.github, &user.slack_name, &user.slack_id, &user.email, &db::to_tinyint(user.mute_direct_messages) as &dyn ToSql, &user.id],
        ).map_err(|e| format_err!("Error updating user {}: {}", user.github, e))?;

        Ok(())
    }

    pub fn delete(&mut self, user_id: i32) -> Result<()> {
        let conn = self.db.connect()?;
        conn.execute("DELETE from users where id = ?1", &[&user_id])
            .map_err(|e| format_err!("Error deleting user {}: {}", user_id, e))?;

        Ok(())
    }

    pub fn slack_user_name(&self, github_name: &str) -> Option<String> {
        self.lookup_info(github_name).map(|u| u.slack_name)
    }

    pub fn slack_direct_message(&self, github_name: &str) -> Option<SlackChannel> {
        self.lookup_info(github_name).and_then(|u| {
            if u.mute_direct_messages {
                None
            } else if !u.slack_id.is_empty() {
                Some(SlackChannel::new(&u.slack_id, &u.slack_name))
            } else {
                Some(SlackChannel::new(&mention(&u.slack_name), &u.slack_name))
            }
        })
    }

    pub fn get_all(&self) -> Result<Vec<UserInfo>> {
        let conn = self.db.connect()?;
        let mut stmt = conn.prepare(
            "SELECT id, slack_name, slack_id, email, github_name, mute_direct_messages FROM users ORDER BY github_name",
        )?;
        let found = stmt.query_map([], |row| {
            Ok(UserInfo {
                id: row.get(0)?,
                slack_name: row.get(1)?,
                slack_id: row.get(2)?,
                email: row.get(3)?,
                github: row.get(4)?,
                mute_direct_messages: db::to_bool(row.get(5)?),
            })
        })?;

        let mut users = vec![];
        for user in found {
            users.push(user?);
        }

        Ok(users)
    }

    pub fn lookup_info(&self, github_name: &str) -> Option<UserInfo> {
        match self.do_lookup_info(github_name) {
            Ok(u) => u,
            Err(e) => {
                error!("Error looking up user: {}", e);
                None
            }
        }
    }

    fn do_lookup_info(&self, github_name: &str) -> Result<Option<UserInfo>> {
        let github_name = github_name.to_string();
        let conn = self.db.connect()?;
        let mut stmt = conn.prepare(
            "SELECT id, slack_name, slack_id, email, mute_direct_messages FROM users where github_name = ?1",
        )?;
        let found = stmt.query_map(&[&github_name], |row| {
            Ok(UserInfo {
                id: row.get(0)?,
                slack_name: row.get(1)?,
                slack_id: row.get(2)?,
                email: row.get(3)?,
                github: github_name.clone(),
                mute_direct_messages: db::to_bool(row.get(4)?),
            })
        })?;

        let user = found.into_iter().flatten().next();
        Ok(user)
    }
}

fn mention(username: &str) -> String {
    format!("@{}", username)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempdir::TempDir;

    fn new_test() -> (UserConfig, TempDir) {
        let temp_dir = TempDir::new("users.rs").unwrap();
        let db_file = temp_dir.path().join("db.sqlite3");
        let db = ConfigDatabase::new(&db_file.to_string_lossy()).expect("create temp database");

        (UserConfig::new(db), temp_dir)
    }

    #[test]
    fn test_slack_user_name_no_defaults() {
        let (users, _temp) = new_test();

        assert_eq!(None, users.slack_user_name("joe"));
        assert_eq!(None, users.slack_direct_message("joe"));
    }

    #[test]
    fn test_slack_user_name() {
        let (mut users, _temp) = new_test();

        users.insert("some-git-user", "the-slacker").unwrap();

        assert_eq!(
            Some("the-slacker".into()),
            users.slack_user_name("some-git-user")
        );
        assert_eq!(
            Some(SlackChannel::new("@the-slacker", "the-slacker")),
            users.slack_direct_message("some-git-user")
        );
        assert_eq!(None, users.slack_user_name("some.other.user"));
        assert_eq!(None, users.slack_direct_message("some.other.user"));
    }

    #[test]
    fn test_slack_user_name_with_id() {
        let (mut users, _temp) = new_test();

        let info = UserInfo::new("some-git-user", "the-slacker", "1234", "");
        users.insert_info(&info).unwrap();

        assert_eq!(
            Some("the-slacker".into()),
            users.slack_user_name("some-git-user")
        );
        assert_eq!(
            Some(SlackChannel::new("1234", "the-slacker")),
            users.slack_direct_message("some-git-user")
        );
    }

    #[test]
    fn test_mention() {
        assert_eq!("@me", mention("me"));
    }
}
