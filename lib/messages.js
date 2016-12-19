var users = require('./users');
var repos = require('./repos');
var util = require('./util');
var peaceAndQuiet = require('./peace-and-quiet');

var g_warned_repos = {};

exports.assignees = function(pullRequest, repo) {
    if (!pullRequest) {
        return [];
    }

    if (pullRequest.assignees) {
        return pullRequest.assignees.map(function(a) {
            return users.slackUserRef(a.login, repo);
        });
    } else if (pullRequest.assignee) { // older api -- github enterprise
        return [ users.slackUserRef(pullRequest.assignee.login, repo) ];
    }

    return [];
}

exports.assigneesStr = function(pullRequest, repo) {
    return exports.assignees(pullRequest, repo).join(', ');
}

exports.newMessenger = function(slack) {
    return {
        sendToAll: function(msg, attachments, item, repo, sender) {
            exports.sendToAll(slack, msg, attachments, item, repo, sender);
        },

        sendToChannel: function(msg, attachments, item, repo, sender) {
            exports.sendToChannel(slack, msg, attachments, item, repo, sender);
        },

        sendToOwner: function(msg, attachments, item, repo) {
            exports.sendToOwner(slack, msg, attachments, item, repo);
        },
    }
};


function sendToSlackbots(slack, slackbots, msg, attachments) {
    slackbots.forEach(function(name) {
        if (!peaceAndQuiet.desiresPeaceAndQuiet(name)) {
            slack.send({
                text: msg,
                attachments: attachments,
                channel: name,
            });
        }
    });
}

exports.sendToAll = function(slack, msg, attachments, item, repo, sender) {
    exports.sendToChannel(slack, msg, attachments, item, repo, sender);

    var slackbots = exports.assignees(item, repo);

    if (item.user) {
        var owner = users.slackUserRef(item.user.login, repo);
        if (slackbots.indexOf(owner) < 0) {
            slackbots.push(owner);
        }
    }

    // make sure we do not send private message to author
    if (sender) {
        var author = users.slackUserRef(sender.login, repo);
        var authorIndex = slackbots.indexOf(author);
        if (authorIndex >= 0) {
            slackbots.splice(authorIndex, 1);
        }
    }

    // send the messages
    sendToSlackbots(slack, slackbots, msg, attachments);
}

exports.sendToOwner = function(slack, msg, attachments, item, repo) {
    exports.sendToChannel(slack, msg, attachments, item, repo);

    var slackbots = [];

    if (item.user) {
        var owner = users.slackUserRef(item.user.login, repo);
        if (slackbots.indexOf(owner) < 0) {
            slackbots.push(owner);
        }
    }

    // send the messages
    sendToSlackbots(slack, slackbots, msg, attachments);
}

exports.sendToChannel = function(slack, msg, attachments, item, repo, sender) {
    if (repo) {
        msg = msg + ' (' + util.makeLink( repo.html_url, repo.full_name) + ')';

        // send to default channel only if configured
        var repoChannel = repos.getChannel(repo);
        if (repoChannel) {
            if (!peaceAndQuiet.desiresPeaceAndQuiet(repoChannel)) {
                slack.send({
                    text: msg,
                    attachments: attachments,
                    channel: repoChannel,
                });
            }
        } else {
            if (!g_warned_repos[repo.html_url]) {
                console.warn("Warning: No repo configured for '" + repo.html_url + "'");
                g_warned_repos[repo.html_url] = true;
            }
        }
    } else {
        console.error("`sendToChannel` called without a repo!");
    }
}
