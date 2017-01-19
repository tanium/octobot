#!/bin/sh

set -e

source $HOME/.cargo/env

git pull --ff-only
cargo test
cargo install --force
# note: may want to setup nopasswd sudo rule for this reload.
sudo systemctl reload supervisor
