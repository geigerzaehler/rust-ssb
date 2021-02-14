//! Box Stream server that accepts one connection from a client.
//!
//! Data received on stdin is sent to the client. Data receveived from the client is printed to
//! stdout.

use futures::prelude::*;

const SERVER_IDENTITY_SEED: [u8; 32] = [5u8; 32];
const SOCKET_ADDR: &str = "localhost:5555";
const NETWORK_IDENTIFIER: [u8; 32] = [1u8; 32];

#[async_std::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let server_identity_seed =
        sodiumoxide::crypto::sign::ed25519::Seed::from_slice(&SERVER_IDENTITY_SEED).unwrap();
    let server_identity = sodiumoxide::crypto::sign::keypair_from_seed(&server_identity_seed);

    let listener = async_std::net::TcpListener::bind(SOCKET_ADDR).await?;
    let (stream, _) = listener.accept().await?;
    println!("Started server with identity {:?}", server_identity.0);
    let server =
        ssb_box_stream::Server::new(&NETWORK_IDENTIFIER, &server_identity.0, &server_identity.1);
    let (mut sender, mut receiver, client_key) = server.accept(stream).await?;
    println!("Connected to client {:?}", client_key);

    let receive_task = async_std::task::spawn(async move {
        while let Some(data) = receiver.try_next().await.unwrap() {
            println!("<- {}", String::from_utf8_lossy(&data));
        }
        println!("client stopped sending");
    });

    let send_task = async_std::task::spawn(async move {
        let stdin = async_std::io::BufReader::new(async_std::io::stdin());
        let mut lines = stdin.lines();
        while let Some(line) = lines.try_next().await.unwrap() {
            sender.send(line.into()).await.unwrap()
        }
        sender.close().await.unwrap();
    });

    futures::join!(send_task, receive_task);
    Ok(())
}
