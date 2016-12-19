var url = require('url');

var users = require('./users');
var messages = require('./messages');
var util = require('./util');

exports.pingHandler = function(messenger, githubAPI) {
    return function(data) {
        return 200;
    }
}

exports.commitCommentHandler = function(messenger, githubAPI) {
    return function(data) {
        if (data.action == 'created') {
            var commit = data.comment.commit_id.substr(0, 7);
            var commit_url = data.repository.html_url + '/commit/' + data.comment.commit_id;

            var msg = 'Comment on "' + data.comment.path + '" (' + util.makeLink(commit_url, commit) + ')';
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

exports.pullRequestHandler = function(messenger, githubAPI) {
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

        if (data.action == 'labeled') {
            mergePullRequest(messenger, githubAPI, data.pull_request, data.repository, data.label);
        } else if (verb == 'merged') {
            mergePullRequestAllLabels(messenger, githubAPI, data.pull_request, data.repository);
        }

        return 200;
    }
}

function pullRequestComment(messenger, data, commentObj) {
    // don't send empty comment messages.
    if (!commentObj.body || commentObj.body.trim().length == 0 ) {
        return;
    }

    var msg = 'Comment on "' + util.makeLink(data.pull_request.html_url, data.pull_request.title) + '"';
    var attachments = [{
        title: users.slackUserName(commentObj.user.login, data.repository) + ' said:',
        title_link: commentObj.html_url,
        text: commentObj.body,
    }];

    messenger.sendToAll(msg, attachments, data.pull_request, data.repository, data.sender);
}

function mergePullRequestAllLabels(messenger, githubAPI, pullRequest, repo, label) {
    if (!pullRequest.merged) {
        return;
    }
    var host = url.parse(repo.html_url).host;

    var attachments = [{
        title: 'Source PR: #' + pullRequest.number + ': "' + pullRequest.title + '"',
        title_link: pullRequest.html_url,
    }];

    githubAPI.getPullRequestLabels(host, repo.owner.login, repo.name, pullRequest.number).then(function(result) {
        result.data.forEach(function(label) {
            mergePullRequest(messenger, githubAPI, pullRequest, repo, label);
        });

    }).catch(function(e) {
        sendGithubSlackError(messenger, e, "Error getting Pull Request labels", attachments, pullRequest, repo);
    });
}

function mergePullRequest(messenger, githubAPI, pullRequest, repo, label) {
    if (!pullRequest.merged || !label) {
        return;
    }

    var host = url.parse(repo.html_url).host;

    // TODO: could eventually make this configurable per `repo`
    var match = /backport-([\d\.]+)/i.exec(label.name);
    if (!match) {
        return;
    }
    var targetBranch = "release/" + match[1];

    var attachments = [{
        title: 'Source PR: #' + pullRequest.number + ': "' + pullRequest.title + '"',
        title_link: pullRequest.html_url,
    }];

    githubAPI.createMergePR(host, repo.owner.login, repo.name, pullRequest.number, targetBranch).then(function(mergePR) {
        messenger.sendToOwner("Created merge Pull Request", attachments, pullRequest, repo);

    }).catch(function(e) {
        sendGithubSlackError(messenger, e, "Error creating merge Pull Request", attachments, pullRequest, repo);
    });
}

function sendGithubSlackError(messenger, e, msg, attachments, item, repo) {
    attachments[0].color = 'danger';
    if (e && e.response && e.response.data && e.response.data.errors) {
        attachments[0].text = "Failed with HTTP " + e.response.status + ": " + e.response.data.errors.map(function(e) { return e.message }).join("\n");
    } else if (e && e.message) {
        attachments[0].text = String(e.message);
    } else {
        attachments[0].text = String(e);
    }

    messenger.sendToOwner(msg, attachments, item, repo);
}


exports.pullRequestCommentHandler = function(messenger, githubAPI) {
    return function(data) {
        if (data.action == "created") {
            pullRequestComment(messenger, data, data.comment);
        }
        return 200;
    }
}

exports.pullRequestReviewHandler = function(messenger, githubAPI) {
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
                // just a comment. should just be handled by regular comment handler.
                pullRequestComment(messenger, data, data.review);
                return 200;
            }

            var user = users.slackUserName(data.review.user.login, data.repository);

            var msg = user + ' ' + actionMsg + ' PR "' + util.makeLink(data.pull_request.html_url, data.pull_request.title) + '"';
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

exports.issueCommentHandler = function(messenger, githubAPI) {
    return function(data) {
        // only notify on new comments
        if (data.action == "created") {
            var msg = 'Comment on "' + util.makeLink(data.issue.html_url, data.issue.title) + '"';
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

exports.statusHandler = function(messenger, githubAPI) {
    return function(data) {
        var msg = 'Status set to ' + data.state + ' on "' + util.makeLink(data.commit.html_url, data.commit.commit.message) + '"';
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

exports.pushHandler = function(messenger, githubAPI) {
    return function(data) {

        return 200;
    }
}

