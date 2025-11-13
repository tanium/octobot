use octobot_lib::db::migrations::{Migration, sql};

pub fn all_migrations() -> Vec<Box<dyn Migration>> {
    vec![sql(r#"
    create table processed_webhooks (
      guid varchar not null,
      timestamp integer not null,
      PRIMARY KEY( guid )
    );

    create table last_checked_webhook (
      id integer PRIMARY KEY CHECK (id = 0),
      guid varchar not null,
      delivered_at integer not null
    );
    "#)]
}
