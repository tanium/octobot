#!/bin/bash

set -e

if [[ "$1" = "octobot" ]] || [[ "$1" = "octobot-passwd" ]]; then
  umask 0077
  chown -R octobot /data
  exec gosu octobot $@
fi

exec $@
