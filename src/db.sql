create table if not exists users (
  id integer not null,
  github_name varchar not null,
  slack_name varchar not null,
  UNIQUE( github_name ),
  PRIMARY KEY( id )
);

create table if not exists repos (
  id integer not null,
  repo varchar not null,
  channel varchar not null,
  force_push_notify tinyint,
  branches varchar,
  jira_projects varchar,
  jira_versions_enabled tinyint,
  version_script varchar,
  release_branch_prefix varchar,

  UNIQUE( repo ),
  PRIMARY KEY( id )
);
