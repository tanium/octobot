
var Slack = require('node-slack');
var express = require('express');
var bodyParser = require('body-parser')


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

function newServer(slack) {
    var app = express();

    app.use(bodyParser.json())

    var handlers = {};

    var newHandler = function(handler) {
        return handler(slack);
    };

    handlers['ping'] = newHandler(pingHandler);
    handlers['commit_comment'] = newHandler(commitCommentHandler);
    handlers['pull_request'] = newHandler(pullRequestHandler);
    handlers['pull_request_review_comment'] = newHandler(pullRequestCommentHandler);
    handlers['status'] = newHandler(statusHandler);

    app.post('/', function (req, res) {
        var event = req.headers['x-github-event'];
        if (!handlers[event]) {
            res.send('Unhandled event: ' + handlers[event]);
            res.end();
            return;
        }

        res.sendStatus(handlers[event](req.body));
        res.end();
    });
    return app;
}


function pingHandler(slack) {
    return function(data) {
        slack.send({
            text: 'Howdy ping!',
            channel: '@matt.hauck',
        });
        return 200;
    }
}

function commitCommentHandler(slack) {
    return function(data) {
        return 200;
    }
}

function pullRequestHandler(slack) {
    return function(data) {
        return 200;
    }
}

function pullRequestCommentHandler(slack) {
    return function(data) {
        return 200;
    }
}

function statusHandler(slack) {
    return function(data) {
        return 200;
    }
}

if (require.main === module) {
    main();
}
