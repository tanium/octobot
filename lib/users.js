
var fs = require('fs');
var url = require('url');

// map: { github-host -> { github-username -> slack-username } }
var users = {};

// just load on startup
if (fs.existsSync('users.json')) {
    users = JSON.parse(fs.readFileSync('users.json'));
}

function lookupUser(login, repo) {
    if (repo) {
        var host = url.parse(repo.clone_url).host;
        var usersForHost = users[host];
        if (usersForHost) {
            var mappedLogin = usersForHost[login];
            if (mappedLogin) {
                return mappedLogin;
            }
        }
    }

    return null;
}

exports.slackUserName = function(login, repo) {
    var mappedLogin = lookupUser(login, repo);
    if (mappedLogin) {
        return mappedLogin;
    }

    // our slack convention is to use '.' but github replaces dots with dashes.
    return login.replace('-', '.');
}

exports.mention = function(username) {
  return '@' + username;
}

exports.slackUserRef = function(login, repo) {
  return exports.mention(exports.slackUserName(login, repo));
}

