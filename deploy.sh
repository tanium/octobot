#!/bin/sh

set -e

host_and_user=$1

if [ -z "$host_and_user" ]; then
  echo "Usage: deploy.sh user@host"
  exit 1
fi

set -x

ssh -A $host_and_user "cd octocat-slack; git pull --ff-only; pm2 restart octocat-slack"
