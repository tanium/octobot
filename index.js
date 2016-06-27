
var Slack = require('node-slack');
var express = require('express');
var bufferEq = require('buffer-equal-constant-time');
var crypto = require('crypto');

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
    handlers['status'] = newHandler(statusHandler);

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

function slackUser(login) {
    // our slack convention is to use '.' but github replaces dots with dashes.
    return '@' + login.replace('-', '.');
}

function assignees(pullRequest) {
    if (!pullRequest || !pullRequest.assignees) {
        return [];
    }
    return pullRequest.assignees.map(function(a) {
        return slackUser(a.login);
    });
}

function assigneesStr(pullRequest) {
    return assignees(pullRequest).join(', ');
}

function sendToAll(slack, item, msg, attachments) {
    console.log("Sending message to channel");
    slack.send({
        text: msg,
        attachments: attachments,
    });

    // try to find assignees and send to them
    assignees(item).forEach(function(name) {
        console.log("Sending private message to assignee " + name);
        slack.send({
            text: msg,
            attachments: attachments,
            channel: name,
        });
    });

    // try to send to owner
    if (item.user) {
        var owner = slackUser(item.user.login);
        console.log("Sending private message to owner " + owner);
        slack.send({
            text: msg,
            attachments: attachments,
            channel: owner,
        });
    }

    // try to send to author
    if (item.author) {
        var owner = slackUser(item.author.login);
        console.log("Sending private message to author " + owner);
        slack.send({
            text: msg,
            attachments: attachments,
            channel: owner,
        });
    }

}


function pingHandler(slack) {
    return function(data) {
        return 200;
    }
}

function commitCommentHandler(slack) {
    return function(data) {
        if (data.action == 'created' || data.action == 'edited') {
            var msg = 'Comment on "' + data.comment.path + '" (' + data.comment.commit_id.substr(0, 7) + ')';
            var attachments = [{
                title: slackUser(data.comment.user.login) + ' said:',
                title_link: data.comment.html_url,
                text: data.comment.body,
            }];

            sendToAll(slack, data.comment, msg, attachments);
        }
        return 200;
    }
}

function pullRequestHandler(slack) {
    return function(data) {
        var verb;
        var extra = '';
        if (data.action == 'opened') {
            verb = 'opened';
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
            extra = ' to ' + assigneesStr(data.pull_request);
        } else if (data.action == 'unassigned') {
            verb = 'unassigned';
        }

        if (verb) {
            var msg = 'Pull Request ' + verb + extra;
            var attachments = [{
                title: 'Pull Request #' + data.pull_request.number + ': "' + data.pull_request.title + '"',
                title_link: data.pull_request.html_url,
            }];

            sendToAll(slack, data.pull_request, msg, attachments);
        }

        return 200;
    }
}

function pullRequestCommentHandler(slack) {
    return function(data) {
        if (data.action == "created" || data.action == "edited") {
            var msg = 'Comment on "' + data.pull_request.title + '"';
            var attachments = [{
                title: slackUser(data.comment.user.login) + ' said:',
                title_link: data.comment.html_url,
                text: data.comment.body,
            }];

            sendToAll(slack, data.pull_request, msg, attachments);
        }
        return 200;
    }
}

function issueCommentHandler(slack) {
    return function(data) {
        // only notify on new/edited comments
        if (data.action == "created" || data.action == "edited") {
            var msg = 'Comment on "' + data.issue.title + '"';
            var attachments = [{
                title: slackUser(data.comment.user.login) + ' said:',
                title_link: data.comment.html_url,
                text: data.comment.body,
            }];
            sendToAll(slack, data.issue, msg, attachments);
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

        sendToAll(slack, msg, attachments, data.commit);

        return 200;
    }
}

if (require.main === module) {
    main();
}
