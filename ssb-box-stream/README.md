# ssb-box-stream

[![Build Status](https://travis-ci.org/geigerzaehler/rust-ssb.svg?branch=main)](https://travis-ci.org/geigerzaehler/rust-ssb)

Implementation of [Scuttlebutt][scuttlebutt] [Handshake][handshake] and [Box
Stream][box-stream] to establish a secure authenticated and encrypted
connection between two peers.

## Usage

A simple echo server (see [`examples/echo_server.rs`][echo-server])

```rust
let server_identity = sodiumoxide::crypto::sign::gen_keypair().unwrap();

let listener = async_std::net::TcpListener::bind("localhost:5555").await?;
let (stream, _) = listener.accept().await?;
let server =
    ssb_box_stream::Server::new(&NETWORK_IDENTIFIER, &server_identity.0, &server_identity.1);
let (mut sender, mut receiver, client_key) = server.accept(stream).await?;
println!("Connected to client {:?}", client_key);

while let Some(data) = receiver.try_next().await? {
    println!("<- {}", String::from_utf8_lossy(&data));
    sender.send(data).await?
}

sender.close().await?
```

A client (see [`examples/client.rs`][client]).

```rust
// This needs to match the server identity keypair
let server_identity_pk = sodiumoxide::crypto::sign::gen_keypair().0;
let client_identity = sodiumoxide::crypto::sign::gen_keypair();

let stream = async_std::net::TcpStream::connect("localhost:5555").await?;

let client = ssb_box_stream::Client::new(
    &NETWORK_IDENTIFIER,
    &server_identity_pk,
    &client_identity.0,
    &client_identity.1,
);

let (mut sender, _receiver) = client.connect(stream).await?;
sender.send(Vec::from(b"hello world")).await?;
```

[scuttlebutt]: https://scuttlebutt.nz/
[handshake]: https://ssbc.github.io/scuttlebutt-protocol-guide/#handshake
[box-stream]: https://ssbc.github.io/scuttlebutt-protocol-guide/#handshake
[echo-server]: ./examples/echo_server.rs
[client]: ./examples/client.rs
