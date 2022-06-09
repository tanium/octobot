use octobot_lib::db::migrations::{sql, Migration};

pub fn all_migrations() -> Vec<Box<dyn Migration>> {
    vec![sql(r#"
    create table pull_request_threads (
      guid varchar not null,
      channel varchar not null,
      thread varchar not null,
      timestamp integer not null,
      PRIMARY KEY( guid )
    );
    "#)]
}
