//! Box stream client that communicates with a server over stdin/stdout.
use async_std::net::TcpStream;
use futures::prelude::*;
use sodiumoxide::crypto::{hash::sha256, sign};
use ssb::{box_stream, handshake};
use std::convert::TryFrom;
use std::io::Write as _;

#[async_std::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = sodiumoxide::init();

    let network_identifier = hash(&"app_key");
    let client_identity_seed = sign::Seed::from_slice(&hash(&"client")).unwrap();
    let server_identity_seed = sign::Seed::from_slice(&hash(&"server")).unwrap();
    let client_identity = sign::keypair_from_seed(&client_identity_seed);
    let server_identity_pk = sign::keypair_from_seed(&server_identity_seed).0;

    let client = handshake::Client::new(
        &network_identifier,
        &server_identity_pk,
        &client_identity.0,
        &client_identity.1,
    );

    let mut stream = TcpStream::connect("localhost:3000").await?;

    let box_stream_params = client.handshake(&mut stream).await?;

    println!("Connected");

    let (mut encrypt, mut decrypt) = box_stream(stream, box_stream_params);

    let stdin = async_std::io::stdin();

    loop {
        print!("-> ");
        std::io::stdout().flush()?;
        let mut line = String::new();
        let read = stdin.read_line(&mut line).await?;
        if read == 0 {
            println!("Saying goodbye");
            break;
        }
        encrypt.send(line.trim().into()).await?;
        match decrypt.next().await {
            Some(result) => println!("<- {}", String::from_utf8_lossy(&result?)),
            None => {
                println!("Server said goodbye");
                break;
            }
        }
    }

    encrypt.close().await?;
    Ok(())
}

pub fn hash(data: impl AsRef<[u8]>) -> [u8; 32] {
    <[u8; 32]>::try_from(sha256::hash(data.as_ref()).as_ref()).unwrap()
}
