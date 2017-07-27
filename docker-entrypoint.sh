#!/bin/bash

set -e

if [[ "$1" = "octobot" ]] || [[ "$1" = "octobot-passwd" ]]; then
  umask 0077
  chown -R octobot /data
  exec gosu octobot sh -c 'rm /data/foo || echo "no foo"; touch /data/foo; ls -l /data/foo'
  exec gosu octobot $@
fi

exec $@
