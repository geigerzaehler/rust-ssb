# Developer Guide

## Tests

Tests are run with `cargo test --all-features`. The integration tests require
two support servers to run test against.

The SSB server that can be started as a Docker container with
`./tests/ssb-server.sh`.

The Muxrpc test suite server requires NodeJs and can be started with `node
muxrpc-test-suite/server.js` after running `npm install` in the
`muxrpc-test-suite` directory.
