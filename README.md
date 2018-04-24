octobot
=======
[![Build Status](https://travis-ci.org/tanium/octobot.svg?branch=master)](https://travis-ci.org/tanium/octobot)

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

- Install docker
- Run `build.py`

This will result in docker image called `octobot:latest` that you can deploy as follows:

    docker run --restart=always --privileged -d  -p 80:3000 -p 443:3001 \
           -v /path/to/host/storage/:/data --name octobot --hostname octobot octobot:latest

* Make sure that whatever path you map `/data` to is a persistent location since this is where configuration is stored.
* Create a `config.toml` file in this location before deploying (see below).

### Configuration

There is one main config file to know about. Hopefully this examples will be sufficiently explanatory:

config.toml

    [main]
    slack_webhook_url = "<slack webhook URL>"
    clone_root_dir = "/home/octobot/repos"
    ssl_cert_file = "/data/ssl.crt"
    ssl_key_file = "/data/ssl.key"
    listen_addr = "0.0.0.0:3000"
    listen_addr_ssl = "0.0.0.0:3001"

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
    resolved_states = [ "Resolved", "Done" ]
    fixed_resolutions = [ "Fixed", "Done" ]
    fix_version_field = "fixVersions"


For the octobot github user token, you will need to:

- Create and octobot developer app in github for your organization
- Create an octobot user in github
- Run the following command to get a token:

        curl -u octobot https://git.company.com/api/v3/authorizations \
             -d '{"scopes": ["repo"], "client_id": "<app id>", "client_secret": "<app secret>"}'

- Grab the "token" value and put it in the config file.

  :rotating_light: **Warning** :rotating_light:

  This token has read/write access to your code. Guard it carefully and make sure config.toml is only readable by root.

### Web UI

To configure repositories and users, you will need to login to octobot's web UI, for which you will need to create a password.

       octobot-passwd <path/to/config.toml> <admin username>

This does not need to be run inside the docker container since it just modifies the configuration file.

### SSL config

It is highly recommended to enable SSL.

When SSL is enabled, the plain HTTP port will always redirect to HTTPS.

It should be noted that the SSL implementation is very particular about certificates and SNI.
Make sure your SSL certificate has a subjectAltName that matches your octobot's hostname exactly.

Addenda
-------

### Tested configurations

Tested with GitHub Enterprise as well as GitHub.com and primarily with on-premise JIRA.

### License

See [LICENSE.txt](LICENSE.txt) for details.
