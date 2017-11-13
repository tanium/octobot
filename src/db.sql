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
  force_push_notify tinyint not null,
  force_push_reapply_statuses varchar not null,
  branches varchar not null,
  jira_projects varchar not null,
  jira_versions_enabled tinyint not null,
  version_script varchar not null,
  release_branch_prefix varchar not null,

  UNIQUE( repo, branches ),
  PRIMARY KEY( id )
);
