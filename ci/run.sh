#!/usr/bin/env bash

set -euo pipefail

source ~/.nvm/nvm.sh

nvm install v12
nvm use v12
cargo clippy --locked --all-targets --all-features -- --deny warnings

(
  cd ssb
  DETACH=true ./tests/ssb-server.sh
  (
      cd muxrpc-test-suite
      yarn --frozen-lockfile install
  )
  node muxrpc-test-suite/server.js &
)
RUST_LOG=debug RUST_BACKTRACE=1 cargo test --all-targets --all-features
