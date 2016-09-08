
var Slack = require('node-slack');
var express = require('express');
var bufferEq = require('buffer-equal-constant-time');
var crypto = require('crypto');
var users = require('./lib/users');

function initSlack() {
    if (process.env.HOOK_URL) {
        hookURL = process.env.HOOK_URL;
    } else {
        console.log("Must configure HOOK_URL!");
        process.exit(1);
    }
    return new Slack(hookURL);
}


function main()  {
    var slack = initSlack();
    var app = newServer(slack);

    app.set('port', (process.env.PORT || 5000));
    app.listen(app.get('port'), function() {
        console.log('Node app is running on port', app.get('port'));
    });
}

function hasValidSignature(sig, body) {
    var secret = process.env.GITHUB_SECRET;
    if (!secret) {
        console.error("GITHUB_SECRET is not configured");
        return true;
    }
    if (!sig) {
        console.error("Request is unsigned");
        return false;
    }

    var computedSig = 'sha1=' + crypto.createHmac('sha1', secret).update(body).digest('hex');
    return bufferEq(new Buffer(sig), new Buffer(computedSig));
}

function newServer(slack) {
    var app = express();

    // concatenate raw body
    app.use(function(req, res, next) {
        var data = '';
        req.setEncoding('utf8');
        req.on('data', function(chunk) {
            data += chunk;
        });
        req.on('end', function() {
            req.rawBody = data;
            next();
        });
    });

    var handlers = {};

    var newHandler = function(handler) {
        return handler(slack);
    };

    handlers['ping'] = newHandler(pingHandler);
    handlers['commit_comment'] = newHandler(commitCommentHandler);
    handlers['pull_request'] = newHandler(pullRequestHandler);
    handlers['pull_request_review_comment'] = newHandler(pullRequestCommentHandler);
    handlers['issue_comment'] = newHandler(issueCommentHandler);
    // disable status updates for now -- too noisy
    //handlers['status'] = newHandler(statusHandler);

    app.post('/', function (req, res) {
        var rawBody = req.rawBody;
        var sig = req.headers['x-hub-signature'];
        if (!hasValidSignature(sig, rawBody)) {
            console.error("Invalid signature");
            res.sendStatus(403);
            res.end();
            return;
        }

        var event = req.headers['x-github-event'];
        if (!handlers[event]) {
            res.send('Unhandled event: ' + handlers[event]);
            res.end();
            return;
        }

        var json = JSON.parse(rawBody)

        res.sendStatus(handlers[event](json));
        res.end();
    });
    return app;
}


function assignees(pullRequest, repo) {
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


function assigneesStr(pullRequest, repo) {
    return assignees(pullRequest, repo).join(', ');
}

function sendToAll(slack, msg, attachments, item, repo) {
    if (repo) {
        msg = msg + ' (<' + repo.html_url + '|' + repo.full_name + '>)';
    }

    console.log("Sending message to channel");
    slack.send({
        text: msg,
        attachments: attachments,
    });

    var slackbots = assignees(item, repo);

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


function pingHandler(slack) {
    return function(data) {
        return 200;
    }
}

function commitCommentHandler(slack) {
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

            sendToAll(slack, msg, attachments, data.comment, data.repository);
        }
        return 200;
    }
}

function pullRequestHandler(slack) {
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
            extra = ' to ' + assigneesStr(data.pull_request, data.repository);
        } else if (data.action == 'unassigned') {
            verb = 'unassigned';
        }

        if (verb) {
            var msg = 'Pull Request ' + verb + extra;
            var attachments = [{
                title: 'Pull Request #' + data.pull_request.number + ': "' + data.pull_request.title + '"',
                title_link: data.pull_request.html_url,
            }];

            sendToAll(slack, msg, attachments, data.pull_request, data.repository);
        }

        return 200;
    }
}

function pullRequestCommentHandler(slack) {
    return function(data) {
        if (data.action == "created" || data.action == "edited") {
            var msg = 'Comment on "<' + data.pull_request.html_url + '|' + data.pull_request.title + '>"';
            var attachments = [{
                title: users.slackUserName(data.comment.user.login, data.repository) + ' said:',
                title_link: data.comment.html_url,
                text: data.comment.body,
            }];

            sendToAll(slack, msg, attachments, data.pull_request, data.repository);
        }
        return 200;
    }
}

function issueCommentHandler(slack) {
    return function(data) {
        // only notify on new/edited comments
        if (data.action == "created" || data.action == "edited") {
            var msg = 'Comment on "<' + data.issue.html_url + '|' + data.issue.title + '>"';
            var attachments = [{
                title: users.slackUserName(data.comment.user.login, data.repository) + ' said:',
                title_link: data.comment.html_url,
                text: data.comment.body,
            }];
            sendToAll(slack, msg, attachments, data.issue, data.repository);
        }

        return 200;
    }
}

function statusHandler(slack) {
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

        sendToAll(slack, msg, attachments, data.commit, data.repository);

        return 200;
    }
}

if (require.main === module) {
    main();
}
