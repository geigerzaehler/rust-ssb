//! Box stream echo server
use async_std::net::{TcpListener, TcpStream};
use futures::prelude::*;
use sodiumoxide::crypto::{hash::sha256, sign};
use ssb::{box_stream, handshake};
use std::convert::TryFrom;

#[async_std::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("0.0.0.0:3000").await?;
    loop {
        let (stream, _) = listener.accept().await?;
        handle_connection(stream);
    }
}

fn handle_connection(mut stream: TcpStream) {
    async_std::task::spawn(async move {
        let network_identifier = hash(&"app_key");
        let server_identity =
            sign::keypair_from_seed(&sign::Seed::from_slice(&hash(&"server")).unwrap());
        let server =
            handshake::Server::new(&network_identifier, &server_identity.0, &server_identity.1);
        let box_stream_params = server.handshake(&mut stream).await.unwrap();
        println!("Accepted client");

        let (mut encrypt, mut decrypt) = box_stream(stream, box_stream_params);

        encrypt.send(b"hello".to_vec()).await.unwrap();
        loop {
            let next = decrypt.try_next().await.unwrap();
            match next {
                Some(value) => {
                    println!("RECV {}", String::from_utf8_lossy(&value));
                    encrypt.send(value).await.unwrap();
                }
                None => {
                    println!("END");
                    break;
                }
            }
        }
    });
}

fn hash(data: impl AsRef<[u8]>) -> [u8; 32] {
    <[u8; 32]>::try_from(sha256::hash(data.as_ref()).as_ref()).unwrap()
}
