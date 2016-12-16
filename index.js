#!/usr/bin/env node

"use strict";

var Slack = require('node-slack');
var express = require('express');
var bufferEq = require('buffer-equal-constant-time');
var crypto = require('crypto');
var Q =  require('q');
var handlers = require('./lib/handlers');
var messages = require('./lib/messages');

var reqjs = require('requirejs');
reqjs.config({
    nodeRequire: require,
    paths: {
      "approve.js": "node_modules/approve.js",
    },
});

// enable better Q stack traces -- comes with performance hit
// Q.longStackSupport = true;

function initSlack() {
    var hookURL;
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

    var messenger = messages.newMessenger(slack);

    var newHandler = function(handler) {
        return handler(messenger);
    };

    var all_handlers = {};
    all_handlers['ping'] = newHandler(handlers.pingHandler);
    all_handlers['commit_comment'] = newHandler(handlers.commitCommentHandler)
    all_handlers['pull_request'] = newHandler(handlers.pullRequestHandler);
    all_handlers['pull_request_review_comment'] = newHandler(handlers.pullRequestCommentHandler);
    all_handlers['pull_request_review'] = newHandler(handlers.pullRequestReviewHandler)
    all_handlers['issue_comment'] = newHandler(handlers.issueCommentHandler);
    // disable status updates for now -- too noisy
    //all_handlers['status'] = newHandler(handlers.statusHandler);

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
        if (!all_handlers[event]) {
            res.send('Unhandled event: ' + event);
            res.end();
            return;
        }

        var json = JSON.parse(rawBody)

        res.sendStatus(all_handlers[event](json));
        res.end();
    });
    return app;
}

if (require.main === module) {
    main();
}
