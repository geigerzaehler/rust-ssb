#!/usr/bin/env bash

set -euo pipefail

source ~/.nvm/nvm.sh

nvm install v12
nvm use v12
cargo clippy --locked --all-targets -- --deny clippy::all --deny warnings
DETACH=true ./tests/ssb-server.sh
(
    cd muxrpc-test-suite
    yarn --frozen-lockfile install
)
node muxrpc-test-suite/server.js &
cargo test --all-targets