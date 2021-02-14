/*!
Implementation of [Scuttlebutt][scuttlebutt] [Handshake][handshake] and [Box
Stream][box-stream] to establish a secure authenticated and encrypted
connection between two peers.

[![Build Status](https://travis-ci.org/geigerzaehler/rust-ssb.svg?branch=main)](https://travis-ci.org/geigerzaehler/rust-ssb)


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
*/
use futures::prelude::*;

mod cipher;
mod crypto;
mod decrypt;
mod encrypt;
mod handshake;
mod utils;

pub use cipher::Params as CipherParams;
pub use decrypt::{Decrypt, DecryptError};
pub use encrypt::Encrypt;
pub use handshake::{Client, Error, Server};

/// Take a duplex stream and create a [Sink] for sending encrypted data and a [Stream] for
/// receiving and decrypting data.
pub fn box_stream<Stream: AsyncRead + AsyncWrite + Unpin>(
    stream: Stream,
    params: BoxStreamParams,
) -> (
    Encrypt<futures::io::WriteHalf<Stream>>,
    Decrypt<futures::io::ReadHalf<Stream>>,
) {
    let (raw_reader, raw_writer) = stream.split();
    (
        Encrypt::new(raw_writer, params.send),
        Decrypt::new(raw_reader, params.receive),
    )
}

/// A pair of [CipherParams], one for receiving and decrypting data, the other for encrypting and
/// sending data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoxStreamParams {
    pub receive: crate::cipher::Params,
    pub send: crate::cipher::Params,
}

#[cfg(test)]
mod test {
    use super::*;
    use proptest::prelude::*;

    #[test_strategy::proptest]
    fn crypt_stream(messages: Vec<Vec<u8>>) {
        let _ = sodiumoxide::init();
        async_std::task::block_on(async move {
            let params = crate::cipher::Params::arbitrary();
            let (writer, reader) = async_std::os::unix::net::UnixStream::pair().unwrap();
            let reader = Decrypt::new(reader, params.clone());
            let mut writer = Encrypt::new(writer, params.clone());

            let data = messages.concat();
            let write_handle = async_std::task::spawn(async move {
                for data in messages {
                    writer.send(data).await.unwrap();
                }
                writer.close().await.unwrap();
            });
            let data_read = reader.try_concat().await.unwrap();
            prop_assert_eq!(data_read, data);
            write_handle.await;
            Ok(())
        })?;
    }

    #[test_strategy::proptest]
    fn early_termination(
        #[strategy(proptest::collection::vec(any::<u8>(), 1..30))] data: Vec<u8>,
        cutoff: proptest::sample::Index,
    ) {
        let _ = sodiumoxide::init();
        async_std::task::block_on(async move {
            let params = crate::cipher::Params::arbitrary();
            let (raw_writer, raw_reader) = async_std::os::unix::net::UnixStream::pair().unwrap();
            let cutoff = cutoff.index(data.len());
            let raw_reader = raw_reader.take(cutoff as u64);
            let reader = Decrypt::new(raw_reader, params.clone());
            let mut writer = Encrypt::new(raw_writer, params);

            async_std::task::spawn(async move {
                let _ = writer.send(data).await;
            });

            let items = reader.collect::<Vec<_>>().await;
            let err = items.last().unwrap().as_ref().unwrap_err();
            match err {
                DecryptError::Io(io_error) => {
                    prop_assert_eq!(io_error.kind(), std::io::ErrorKind::UnexpectedEof)
                }
                _ => prop_assert!(false),
            }
            Ok(())
        })?;
    }
}
