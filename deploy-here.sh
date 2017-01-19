#!/bin/sh

set -e

# note: may want to setup nopasswd sudo rule for this reload.
git pull --ff-only && cargo test && cargo install --force && sudo systemctl reload supervisor
