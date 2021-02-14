# Secure Scuttlebut in Rust

[![Build Status](https://travis-ci.org/geigerzaehler/rust-ssb.svg?branch=main)](https://travis-ci.org/geigerzaehler/rust-ssb)

An unfinished implementation of the [Scuttlebut protocol][protocol] in Rust.

## `ssbc`

`ssbc` is a command line client to interact with an SSB server.

```bash
cargo run --bin ssbc -- --help
```

At the moment the functionality is limited but will be extended.

## Features

- [x] [Handshake and box stream](./box_stream)
- [ ] [RPC](https://ssbc.github.io/scuttlebutt-protocol-guide/#rpc-protocol)
- [ ] [Feeds](https://ssbc.github.io/scuttlebutt-protocol-guide/#feeds)
- [ ] [LAN discovery](https://ssbc.github.io/scuttlebutt-protocol-guide/#discovery)
- [ ] [SSB API server](https://scuttlebot.io/apis/scuttlebot/ssb.html)
- [ ] Extend `ssbc` command line tool

[protocol]: https://ssbc.github.io/scuttlebutt-protocol-guide
