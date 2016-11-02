#!/bin/sh

set -e

git pull --ff-only && npm install && npm test && pm2 restart octocat-slack
