extern crate octobot;
#[macro_use]
extern crate serde_derive;
extern crate serde_json;

use std::collections::HashMap;
use std::io::Read;

use octobot::db;
use octobot::errors::*;
use octobot::users;

#[derive(Deserialize, Serialize, Clone)]
struct UserInfoJSON {
    pub github: String,
    pub slack: String,
}

type UserHostMap = HashMap<String, Vec<UserInfoJSON>>;

fn main() {
    if let Err(ref e) = run() {
        use std::io::Write;
        let stderr = &mut ::std::io::stderr();
        let errmsg = "Error writing to stderr";

        writeln!(stderr, "error: {}", e).expect(errmsg);

        for e in e.iter().skip(1) {
            writeln!(stderr, "caused by: {}", e).expect(errmsg);
        }

        ::std::process::exit(1);
    }
}

fn run() -> Result<()> {
    if std::env::args().len() < 3 {
        return Err("Usage: migrate-db <db-file> <users.json>".into());
    }

    let db_file = std::env::args().nth(1).unwrap();
    let users_json = std::env::args().nth(2).unwrap();

    let mut users_db = users::UserConfig::new(db::Database::new(&db_file)?);
    let users_map = load_config(&users_json)?;

    for (_, users) in &users_map {
        for user in users {
            if let Err(e) = users_db.insert(&user.github, &user.slack) {
                println!("Error adding user {}: {}", user.github, e);
            } else {
                println!("Added user: {}", user.github);
            }
        }
    }

    Ok(())
}

fn load_config(file: &str) -> Result<UserHostMap> {
    let mut f = std::fs::File::open(file)?;
    let mut contents = String::new();
    f.read_to_string(&mut contents)?;
    serde_json::from_str(&contents).map_err(|_| Error::from("Error parsing json"))
}
