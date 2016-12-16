#!/usr/bin/env node

"use strict";

function printWrongPass() {
    console.log('wrong pass to avoid hanging on password prompt');
}

try {
    var reqjs = require('requirejs');
    reqjs.config({
        nodeRequire: require,
        paths: {
          "approve.js": "node_modules/approve.js",
        },
    });
    var auth = reqjs('approve.js/lib/node/auth');

    var prompt = String(process.argv[2]);
    var match = /Password for '.*@(.*)'/.exec(prompt);
    if (match) {
        var host = match[1];
        console.log(auth.getTokenData(host).token);
    } else {
        printWrongPass();
    }
} catch (e) {
    printWrongPass();
}
