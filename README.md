octobot
=======

Octobot wants to make your github and slack lives better by triggering
more directed pull request notifications to help pull requests from their
worst nightmare: remaining un-reviewed.

Octobot isn't content to stop there, it also wants to help merge pull requests
to release branches for you. All you have to do is label pull requests with
"backport-1.0" (for example) and he will auto-cherry-pick this PR after it is
merged to "release/1.0" and open up a new PR for you.

Yet still more, octobot also wants to help improve JIRA issue tracking.
If a PR is submitted with jira issues in the title, they will be commented on and
transitioned to in-progress/pending-review. When the PR is merged, they will be
commented on again with the PR title/body, and transitioned to Resolved: Fixed.

Setup
-----

- Install rust: https://www.rustup.rs/
- Install openssl: `apt-get install libssl-dev build-essential`
- Deploy! `deploy-here.sh`


### Configuration

There are three important config files. Hopefully these examples will be sufficiently explanatory:

config.toml

    [main]
    slack_webhook_url = "<slack webhook URL>"
    users_config_file = "/home/octobot/users.json"
    repos_config_file = "/home/octobot/repos.json"
    clone_root_dir = "/home/octobot/repos"

    [github]
    webhook_secret = "<secret for github hook>"
    host = "git.company.com"
    api_token = "<token-for-octobot-user>"

    [jira]
    # required to enable jira support
    host = "jira.company.com"
    username = <jira username>
    password = <jira password>

    # optional. shown here with defaults:
    progress_states = [ "In Progress" ]
    review_states = [ "Pending Review" ]
    resolved_states = [ "Resolved", "Done" ]
    fixed_resolutions = [ "Fixed", "Done" ]


users.json

    {
      "git.company.com": {
        "git-user-name": {
          "slack": "slack-user-name"
        }
      }
    }

repos.json

    {
      "git.company.com": {
        "some-org": {
           "channel": "the-org-reviews"
        },
        "some-org/special-repo": {
           "channel": "special-repo-reviews",
           "force_push_notify": false, // turn off force-push notifications
           "jira_enabled": false, // turn off jira integration
        }
      }
    }

As for the octobot user token, you need to:

- Create and octobot developer app in github
- Create an octobot user in github
- Run the following command to get a token:

        curl -u octobot https://git.company.com/api/v3/authorizations -d '{"scopes": ["repo"], "client_id": "<app id>", "client_secret": "<app secret>"}'

- Grab the "token" value and put it in the config file.

  **Warning**: This token has read/write access to code. Guard it carefully and make sure config.toml is only readable by service account.


### Supervisor

supervisord is a great way to run octobot. The built-in deploy-here.sh
assumes you are running with supervisord. All you have to do is add a
configuration like this to /etc/supervisor/conf.d/octobot.conf:

    [program:octobot]
    command=/home/octobot/.cargo/bin/octobot /home/octobot/config.toml
    user=octobot
    environment=HOME="/home/octobot",USER="octobot",RUST_LOG="info"


