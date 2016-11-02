var users = require('./users');
var messages = require('./messages');

exports.pingHandler = function(messenger) {
    return function(data) {
        return 200;
    }
}

exports.commitCommentHandler = function(messenger) {
    return function(data) {
        if (data.action == 'created' || data.action == 'edited') {
            var commit = data.comment.commit_id.substr(0, 7);
            var commit_url = data.repository.html_url + '/commit/' + data.comment.commit_id;

            var msg = 'Comment on "' + data.comment.path + '" (<' + commit_url + '|' + commit + '>)';
            var attachments = [{
                title: users.slackUserName(data.comment.user.login, data.repository) + ' said:',
                title_link: data.comment.html_url,
                text: data.comment.body,
            }];

            messenger.sendToAll(msg, attachments, data.comment, data.repository, data.sender);
        }
        return 200;
    }
}

exports.pullRequestHandler = function(messenger) {
    return function(data) {
        var verb;
        var extra = '';
        if (data.action == 'opened') {
            verb = 'opened by ' + users.slackUserName(data.pull_request.user.login, data.repository);
        } else if (data.action == 'closed') {
            if (data.pull_request.merged) {
                verb = 'merged';
            } else {
                verb = 'closed';
            }
        } else if (data.action == 'reopened') {
            verb = 'reopened';
        } else if (data.action == 'assigned') {
            verb = 'assigned';
            extra = ' to ' + messages.assigneesStr(data.pull_request, data.repository);
        } else if (data.action == 'unassigned') {
            verb = 'unassigned';
        }

        if (verb) {
            var msg = 'Pull Request ' + verb + extra;
            var attachments = [{
                title: 'Pull Request #' + data.pull_request.number + ': "' + data.pull_request.title + '"',
                title_link: data.pull_request.html_url,
            }];

            messenger.sendToAll(msg, attachments, data.pull_request, data.repository, data.sender);
        }

        return 200;
    }
}

exports.pullRequestCommentHandler = function(messenger) {
    return function(data) {
        if (data.action == "created" || data.action == "edited") {
            var msg = 'Comment on "<' + data.pull_request.html_url + '|' + data.pull_request.title + '>"';
            var attachments = [{
                title: users.slackUserName(data.comment.user.login, data.repository) + ' said:',
                title_link: data.comment.html_url,
                text: data.comment.body,
            }];

            messenger.sendToAll(msg, attachments, data.pull_request, data.repository, data.sender);
        }
        return 200;
    }
}

exports.pullRequestReviewHandler = function(messenger) {
    return function(data) {
        if (data.action == "submitted") {
            var stateMsg, actionMsg, color;
            if (data.review.state === "changes_requested") {
                actionMsg = "requested changes to" ;
                stateMsg = 'Changes Requested';
                color = 'danger';
            } else if (data.review.state === "approved") {
                actionMsg = "approved";
                stateMsg = 'Approved';
                color = 'good';
            } else if (data.review.state === "commented") {
                // ignore. should just be handled by regular comment handler.
                return 200;
            }

            var user = users.slackUserName(data.review.user.login, data.repository);

            var msg = user + ' ' + actionMsg + ' PR "<' + data.pull_request.html_url + '|' + data.pull_request.title + '>"';
            var attachments = [{
                title:  'Review: ' + stateMsg,
                title_link: data.review.html_url,
                text: data.review.body,
                color: color,
            }];

            messenger.sendToAll(msg, attachments, data.pull_request, data.repository, data.sender);
        }
        return 200;

    }
}

exports.issueCommentHandler = function(messenger) {
    return function(data) {
        // only notify on new/edited comments
        if (data.action == "created" || data.action == "edited") {
            var msg = 'Comment on "<' + data.issue.html_url + '|' + data.issue.title + '>"';
            var attachments = [{
                title: users.slackUserName(data.comment.user.login, data.repository) + ' said:',
                title_link: data.comment.html_url,
                text: data.comment.body,
            }];
            messenger.sendToAll(msg, attachments, data.issue, data.repository, data.sender);
        }

        return 200;
    }
}

exports.statusHandler = function(messenger) {
    return function(data) {
        var msg = 'Status set to ' + data.state + ' on "<' + data.commit.html_url + '|' + data.commit.commit.message + '>"';
        var attachments = [{
            title: 'Status: ' + data.context,
            title_link: data.target_url,
            text: data.description,
        }];
        if (data.state === 'failure') {
            attachments[0].color = 'danger';
        } else if (data.state === 'success') {
            attachments[0].color = 'good';
        }

        messenger.sendToAll(msg, attachments, data.commit, data.repository, data.sender);

        return 200;
    }
}



