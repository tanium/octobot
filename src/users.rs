use db::Database;
use errors::*;
use github;

#[derive(Deserialize, Serialize, Clone)]
pub struct UserInfo {
    pub id: i32,
    pub github: String,
    pub slack: String,
}

#[derive(Clone)]
pub struct UserConfig {
    db: Database,
}

impl UserConfig {
    pub fn new(db: Database) -> UserConfig {
        UserConfig { db: db }
    }

    pub fn new_memory() -> Result<UserConfig> {
        let db = Database::new(":memory:")?;
        Ok(UserConfig {
            db: db,
        })
    }

    pub fn insert(&mut self, git_user: &str, slack_user: &str) -> Result<()> {
        let conn = self.db.connect()?;
        conn.execute(
            "INSERT INTO users (github_name, slack_name) VALUES (?1, ?2)",
            &[&git_user, &slack_user],
        ).map_err(|e| Error::from(format!("Error inserting user {}: {}", git_user, e)))?;

        Ok(())
    }

    pub fn update(&mut self, user: &UserInfo) -> Result<()> {
        let conn = self.db.connect()?;
        conn.execute(
            "UPDATE users set github_name = ?1, slack_name = ?2 where id = ?3",
            &[&user.github, &user.slack, &user.id],
        ).map_err(|e| Error::from(format!("Error updating user {}: {}", user.github, e)))?;

        Ok(())
    }

    pub fn delete(&mut self, user_id: i32) -> Result<()> {
        let conn = self.db.connect()?;
        conn.execute(
            "DELETE from users where id = ?1",
            &[&user_id],
        ).map_err(|e| Error::from(format!("Error deleting user {}: {}", user_id, e)))?;

        Ok(())
    }

    // our slack convention is to use '.' but github replaces dots with dashes.
    pub fn slack_user_name<S: Into<String>>(&self, login: S) -> String {
        let login = login.into();
        match self.lookup_name(login.as_str()) {
            Some(name) => name,
            None => login.as_str().replace('-', "."),
        }
    }

    pub fn slack_user_ref<S: Into<String>>(&self, login: S) -> String {
        mention(self.slack_user_name(login.into()))
    }

    pub fn slack_user_names(&self, users: &Vec<github::User>) -> Vec<String> {
        users.iter().map(|a| self.slack_user_name(a.login())).collect()
    }

    fn lookup_name(&self, login: &str) -> Option<String> {
        self.lookup_info(login).map(|u| u.slack)
    }

    pub fn get_all(&self) -> Result<Vec<UserInfo>> {
        let conn = self.db.connect()?;
        let mut stmt = conn.prepare("SELECT id, slack_name, github_name FROM users ORDER BY github_name")?;
        let found = stmt.query_map(&[], |row| {
            UserInfo {
                id: row.get(0),
                slack: row.get(1),
                github: row.get(2),
            }
        })?;

        let mut users = vec![];
        for user in found {
            users.push(user?);
        }

        Ok(users)
    }

    fn lookup_info(&self, github_name: &str) -> Option<UserInfo> {
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
        let mut stmt = conn.prepare("SELECT id, slack_name FROM users where github_name = ?1")?;
        let found = stmt.query_map(&[&github_name], |row| {
            UserInfo {
                id: row.get(0),
                slack: row.get(1),
                github: github_name.clone(),
            }
        })?;

        // kinda ugly....
        let mut user = None;
        for u in found {
            if let Ok(u) = u {
                user = Some(u);
                break;
            }
        }
        Ok(user)
    }
}

pub fn mention<S: Into<String>>(username: S) -> String {
    format!("@{}", username.into())
}

#[cfg(test)]
mod tests {
    extern crate tempdir;

    use super::*;
    use self::tempdir::TempDir;

    #[test]
    fn test_slack_user_name_defaults() {
        let users = UserConfig::new_memory().unwrap();

        assert_eq!("joe", users.slack_user_name("joe"));
        assert_eq!("@joe", users.slack_user_ref("joe"));
        assert_eq!("joe.smith", users.slack_user_name("joe-smith"));
        assert_eq!("@joe.smith", users.slack_user_ref("joe-smith"));
    }

    #[test]
    fn test_slack_user_name() {
        let temp_dir = TempDir::new("users.rs").unwrap();
        let db_file = temp_dir.path().join("db.sqlite3");
        let db = Database::new(&db_file.to_string_lossy()).expect("create temp database");

        let mut users = UserConfig::new(db);
        users.insert("some-git-user", "the-slacker").unwrap();

        assert_eq!("the-slacker", users.slack_user_name("some-git-user"));
        assert_eq!("@the-slacker", users.slack_user_ref("some-git-user"));
        assert_eq!("some.other.user", users.slack_user_name("some.other.user"));
        assert_eq!("@some.other.user", users.slack_user_ref("some.other.user"));
    }

    #[test]
    fn test_mention() {
        assert_eq!("@me", mention("me"));
    }
}
