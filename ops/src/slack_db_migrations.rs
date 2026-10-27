use octobot_lib::db::migrations::{Migration, sql};

pub fn all_migrations() -> Vec<Box<dyn Migration>> {
    vec![
        sql(r#"
    create table pull_request_threads (
      guid varchar not null,
      channel varchar not null,
      thread varchar not null,
      timestamp integer not null,
      PRIMARY KEY( guid, channel )
    );
    "#),
        sql(r#"create index pull_request_threads_timestamp on pull_request_threads (timestamp)"#),
        sql(r#"update pull_request_threads set timestamp = CURRENT_TIMESTAMP"#),
    ]
}
