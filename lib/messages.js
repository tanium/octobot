var users = require('./users');

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

exports.sendToAll = function(slack, msg, attachments, item, repo) {
    if (repo) {
        msg = msg + ' (<' + repo.html_url + '|' + repo.full_name + '>)';
    }

    slack.send({
        text: msg,
        attachments: attachments,
    });

    var slackbots = exports.assignees(item, repo);

    if (item.user) {
        var owner = users.slackUserRef(item.user.login, repo);
        if (slackbots.indexOf(owner) < 0) {
            slackbots.push(owner);
        }
    }

    // make sure we do not send private message to author
    if (item.author) {
        var author = users.slackUserRef(item.author.login, repo);
        var authorIndex = slackbots.indexOf(author);
        if (authorIndex >= 0) {
            slackbots.splice(authorIndex, 1);
        }
    }

    // send direct messages
    slackbots.forEach(function(name) {
        slack.send({
            text: msg,
            attachments: attachments,
            channel: name,
        });
    });
}

