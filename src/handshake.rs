//! Execute the connection handshake on a stream.
//!
//! See the [“Handshake”][protocol-handshake] section of the protocol guide.
//!
//! ```no_run
//! # use ssb::handshake::*;
//! # #[async_std::main]
//! # async fn main () {
//! let mut stream = async_std::net::TcpStream::connect("localhost:3000").await.unwrap();
//! let network_identifier = [0u8; 32];
//! let server_identity_pk = sodiumoxide::crypto::sign::gen_keypair().0;
//! let client_identity = sodiumoxide::crypto::sign::gen_keypair();
//! let client = Client::new(
//!     &network_identifier,
//!     &server_identity_pk,
//!     &client_identity.0,
//!     &client_identity.1,
//! );
//! let box_crypt_params = client.handshake(stream).await;
//! # }
//! ```
//! [protocol-handshake]: https://ssbc.github.io/scuttlebutt-protocol-guide/#handshake

// We allow this to align with the names used in the protocol guide.
#![allow(non_snake_case)]

use futures::io::{AsyncRead, AsyncWrite};
use futures::prelude::*;

use crate::box_stream::{BoxCryptParams, BoxStreamParams};
use crate::crypto;

/// Errors returned when running the handshake protocol.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Failed to read data from remote
    #[error("Failed to read data from remote")]
    ReadFailed(#[source] async_std::io::Error),
    /// Failed to write data to remote
    #[error("Failed to write data to remote")]
    WriteFailed(#[source] async_std::io::Error),

    /// Failed to verify `hello` message from remote
    #[error("Failed to verify `hello` message from remote")]
    HelloMessageInvalid,

    /// Failed to decrypt `authenticate` message
    #[error("Failed to decrypt `authenticate` message")]
    AuthenticateMessageDecryptFailed,
    /// Invalid signature in `authenticate` message
    #[error("Invalid signature in `authenticate` message")]
    AuthenticateSignatureInvalid,

    /// Failed to decrypt `accept` message
    #[error("Failed to decrypt `accept` message")]
    AcceptMessageDecryptFailed,
    /// Invalid signature in `accept` message
    #[error("Invalid signature in `accept` message")]
    AcceptSignatureInvalid,
}
pub struct Client {
    endpoint: Endpoint,
    server_identity: crypto::sign::PublicKey,
}

impl Client {
    pub fn new(
        network_identifier: &[u8; 32],
        server_identity: &crypto::sign::PublicKey,
        identity_pk: &crypto::sign::PublicKey,
        identity_sk: &crypto::sign::SecretKey,
    ) -> Self {
        let (session_pk, session_sk) = crypto::box_::gen_keypair();
        let network_identifier = crypto::auth::Key::from_slice(network_identifier).unwrap();
        Self {
            endpoint: Endpoint {
                identity_pk: *identity_pk,
                identity_sk: identity_sk.clone(),
                session_pk,
                session_sk,
                network_identifier,
            },
            server_identity: *server_identity,
        }
    }

    pub async fn handshake(
        self,
        mut stream: impl AsyncRead + AsyncWrite + Unpin,
    ) -> Result<BoxStreamParams, Error> {
        stream
            .write_all(&self.endpoint.hello_message())
            .await
            .map_err(Error::WriteFailed)?;

        let hello_msg = read_hello_bytes(&mut stream).await?;
        let server_session_pk = self.endpoint.hello_verify(hello_msg)?;
        let authenticate = Authenticate::for_client(&self, &server_session_pk);

        let msg = authenticate_message(&self, &authenticate);
        stream.write_all(&msg).await.map_err(Error::WriteFailed)?;

        let accept = Accept::for_client(&self, authenticate);
        let mut reply = [0u8; 80];
        stream
            .read_exact(&mut reply)
            .await
            .map_err(Error::ReadFailed)?;
        accept_message_verify(&self, &accept, reply)?;

        Ok(box_stream_params(
            &self.endpoint,
            &accept,
            &self.server_identity,
            &server_session_pk,
        ))
    }
}

/// Parameters to execute a handshake from the server side
pub struct Server {
    endpoint: Endpoint,
}

impl Server {
    pub fn new(
        network_identifier: &[u8; 32],
        identity_pk: &crypto::sign::PublicKey,
        identity_sk: &crypto::sign::SecretKey,
    ) -> Self {
        let (session_pk, session_sk) = crypto::box_::gen_keypair();
        let network_identifier = crypto::auth::Key::from_slice(network_identifier).unwrap();
        Self {
            endpoint: Endpoint {
                identity_pk: *identity_pk,
                identity_sk: identity_sk.clone(),
                session_pk,
                session_sk,
                network_identifier,
            },
        }
    }

    /// Execute the handshake protocol for the server.
    pub async fn handshake(
        self,
        mut stream: impl AsyncRead + AsyncWrite + Unpin,
    ) -> Result<BoxStreamParams, Error> {
        let hello_msg = read_hello_bytes(&mut stream).await?;
        let client_session_pk = self.endpoint.hello_verify(hello_msg)?;
        let authenticate = Authenticate::for_server(&self, &client_session_pk);

        stream
            .write_all(&self.endpoint.hello_message())
            .await
            .map_err(Error::WriteFailed)?;

        let mut authenticate_msg = [0u8; 112];
        stream
            .read_exact(&mut authenticate_msg)
            .await
            .map_err(Error::ReadFailed)?;

        let accept = authenticate_message_verify(&self, authenticate, &authenticate_msg)?;

        let accept_message = accept_message(&self, &accept);
        stream
            .write_all(&accept_message)
            .await
            .map_err(Error::WriteFailed)?;

        Ok(box_stream_params(
            &self.endpoint,
            &accept,
            &accept.client_identity_pk,
            &client_session_pk,
        ))
    }
}

async fn read_hello_bytes(mut stream: impl AsyncRead + Unpin) -> Result<[u8; 64], Error> {
    let mut reply = [0u8; 64];
    stream
        .read_exact(&mut reply)
        .await
        .map_err(Error::ReadFailed)?;
    Ok(reply)
}

fn authenticate_message(client: &Client, authenticate: &Authenticate) -> Vec<u8> {
    let msg = authenticate.signature_payload(&client.server_identity);
    let detached_signature_A = crypto::sign::sign_detached(&msg, &client.endpoint.identity_sk);

    let key = authenticate.message_key();
    let msg = [
        detached_signature_A.as_ref(),
        client.endpoint.identity_pk.as_ref(),
    ]
    .concat();
    crypto::secretbox::seal(&msg, &crypto::zero_nonce(), &key)
}

fn authenticate_message_verify(
    server: &Server,
    authenticate: Authenticate,
    cipher_msg: &[u8; 112],
) -> Result<Accept, Error> {
    let key = authenticate.message_key();
    let msg = crypto::secretbox::open(&cipher_msg[..], &crypto::zero_nonce(), &key)
        .map_err(|()| Error::AuthenticateMessageDecryptFailed)?;
    debug_assert!(msg.len() == 96);
    let detached_signature_A = crypto::sign::Signature::from_slice(&msg[0..64]).unwrap();
    let client_identity_pk = crypto::sign::PublicKey::from_slice(&msg[64..96]).unwrap();
    let signature_payload = authenticate.signature_payload(&server.endpoint.identity_pk);
    if crypto::sign::verify_detached(
        &detached_signature_A,
        &signature_payload,
        &client_identity_pk,
    ) {
        let accept = Accept::for_server(
            &server,
            authenticate,
            &client_identity_pk,
            &detached_signature_A,
        );
        Ok(accept)
    } else {
        Err(Error::AuthenticateSignatureInvalid)
    }
}

fn accept_message(server: &Server, shared_secrets: &Accept) -> Vec<u8> {
    let msg = shared_secrets.signature_payload();
    let detached_signature_B = crypto::sign::sign_detached(&msg, &server.endpoint.identity_sk);

    crypto::secretbox::seal(
        detached_signature_B.as_ref(),
        &crypto::zero_nonce(),
        &shared_secrets.message_key(),
    )
}

fn accept_message_verify(
    params: &Client,
    accept: &Accept,
    cipher_msg: [u8; 80],
) -> Result<(), Error> {
    let detached_signature_B_payload =
        crypto::secretbox::open(&cipher_msg, &crypto::zero_nonce(), &accept.message_key())
            .map_err(|()| Error::AcceptMessageDecryptFailed)?;
    let detached_signature_B =
        crypto::sign::Signature::from_slice(&detached_signature_B_payload).unwrap();

    let msg = accept.signature_payload();
    if crypto::sign::verify_detached(&detached_signature_B, &msg, &params.server_identity) {
        Ok(())
    } else {
        Err(Error::AcceptSignatureInvalid)
    }
}

/// Data that identifies an endpoint (client or server) at the start of the handshake.
#[derive(Debug)]
struct Endpoint {
    identity_pk: crypto::sign::PublicKey,
    identity_sk: crypto::sign::SecretKey,
    session_pk: crypto::box_::PublicKey,
    session_sk: crypto::box_::SecretKey,
    network_identifier: crypto::auth::Key,
}

impl Endpoint {
    fn hello_message(&self) -> Vec<u8> {
        [
            crypto::auth::authenticate(self.session_pk.as_ref(), &self.network_identifier).as_ref(),
            self.session_pk.as_ref(),
        ]
        .concat()
    }

    fn hello_verify(&self, msg: [u8; 64]) -> Result<crypto::box_::PublicKey, Error> {
        let tag = crypto::auth::Tag::from_slice(&msg[0..32]).unwrap();
        let payload = &msg[32..64];

        if crypto::auth::verify(&tag, payload, &self.network_identifier) {
            let remote_session_public = crypto::box_::PublicKey::from_slice(payload).unwrap();
            Ok(remote_session_public)
        } else {
            Err(Error::HelloMessageInvalid)
        }
    }
}

/// Data that is shared by the server and client before the client sends the `authenticate` message.
struct Authenticate {
    ab: crypto::box_::SecretKey,
    aB: crypto::box_::SecretKey,
    network_identifier: crypto::auth::Key,
    server_session_pk: crypto::box_::PublicKey,
}

impl Authenticate {
    fn for_client(client: &Client, server_session_pk: &crypto::box_::PublicKey) -> Self {
        let ab = crypto::share_key(&server_session_pk, &client.endpoint.session_sk);

        let aB = crypto::share_key(
            &crypto::sign_to_box_pk(&client.server_identity).unwrap(),
            &client.endpoint.session_sk,
        );

        Self {
            ab,
            aB,
            network_identifier: client.endpoint.network_identifier.clone(),
            server_session_pk: *server_session_pk,
        }
    }
    fn for_server(server: &Server, client_session_pk: &crypto::box_::PublicKey) -> Self {
        let ab = crypto::share_key(client_session_pk, &server.endpoint.session_sk);

        let aB = crypto::share_key(
            &client_session_pk,
            &crypto::sign_to_box_sk(&server.endpoint.identity_sk).unwrap(),
        );

        Self {
            ab,
            aB,
            network_identifier: server.endpoint.network_identifier.clone(),
            server_session_pk: server.endpoint.session_pk,
        }
    }

    /// Returns the key that encrypts the `authenticate` message of the client.
    fn message_key(&self) -> crypto::secretbox::Key {
        let key_data = crypto::hash(
            [
                self.network_identifier.as_ref(),
                self.ab.as_ref(),
                self.aB.as_ref(),
            ]
            .concat(),
        );
        crypto::secretbox::Key::from_slice(&key_data).unwrap()
    }

    /// Returns the payload that is signed by the client and part of the `authenticate` message.
    fn signature_payload(&self, server_identity_pk: &crypto::sign::PublicKey) -> Vec<u8> {
        [
            self.network_identifier.as_ref(),
            server_identity_pk.as_ref(),
            crypto::hash(&self.ab).as_ref(),
        ]
        .concat()
    }
}

/// Data that is shared by the server and client before the server sends the `accept` message.
struct Accept {
    authenticate: Authenticate,
    Ab: crypto::box_::SecretKey,
    detached_signature_A: crypto::sign::Signature,
    client_identity_pk: crypto::sign::PublicKey,
}

impl Accept {
    fn for_client(client: &Client, authenticate: Authenticate) -> Self {
        let Ab = crypto::share_key(
            &authenticate.server_session_pk,
            &crypto::sign_to_box_sk(&client.endpoint.identity_sk).unwrap(),
        );

        let msg = [
            client.endpoint.network_identifier.as_ref(),
            client.server_identity.as_ref(),
            crypto::hash(&authenticate.ab).as_ref(),
        ]
        .concat();
        let detached_signature_A = crypto::sign::sign_detached(&msg, &client.endpoint.identity_sk);

        Self {
            authenticate,
            Ab,
            detached_signature_A,
            client_identity_pk: client.endpoint.identity_pk,
        }
    }

    fn for_server(
        server: &Server,
        authenticate: Authenticate,
        client_identity_pk: &crypto::sign::PublicKey,
        detached_signature_A: &crypto::sign::Signature,
    ) -> Self {
        let Ab = crypto::share_key(
            &crypto::sign_to_box_pk(&client_identity_pk).unwrap(),
            &server.endpoint.session_sk,
        );

        Self {
            authenticate,
            Ab,
            detached_signature_A: *detached_signature_A,
            client_identity_pk: *client_identity_pk,
        }
    }

    /// Returns the key that encrypts the `accept` message of the server.
    fn message_key(&self) -> crypto::secretbox::Key {
        crypto::secretbox::Key::from_slice(&crypto::hash(
            [
                self.authenticate.network_identifier.as_ref(),
                self.authenticate.ab.as_ref(),
                self.authenticate.aB.as_ref(),
                self.Ab.as_ref(),
            ]
            .concat(),
        ))
        .unwrap()
    }

    /// Returns the payload that is signed by the server and part of the `accept` message.
    fn signature_payload(&self) -> Vec<u8> {
        [
            self.authenticate.network_identifier.as_ref(),
            self.detached_signature_A.as_ref(),
            self.client_identity_pk.as_ref(),
            crypto::hash(&self.authenticate.ab).as_ref(),
        ]
        .concat()
    }
}

fn box_stream_params(
    local: &Endpoint,
    accept: &Accept,
    remote_identity_pk: &crypto::sign::PublicKey,
    remote_session_pk: &crypto::box_::PublicKey,
) -> BoxStreamParams {
    BoxStreamParams {
        encrypt: BoxCryptParams {
            key: box_stream_key(&accept, remote_identity_pk),
            nonce: box_stream_nonce(local, remote_session_pk),
        },
        decrypt: BoxCryptParams {
            key: box_stream_key(&accept, &local.identity_pk),
            nonce: box_stream_nonce(local, &local.session_pk),
        },
    }
}

fn box_stream_nonce(
    endpoint: &Endpoint,
    receiver_key: &crypto::box_::PublicKey,
) -> crypto::secretbox::Nonce {
    crypto::secretbox::Nonce::from_slice(
        &crypto::auth::authenticate(receiver_key.as_ref(), &endpoint.network_identifier)
            [0..crypto::secretbox::NONCEBYTES],
    )
    .unwrap()
}

fn box_stream_key(
    accept: &Accept,
    receiver_session_key: &crypto::sign::PublicKey,
) -> crypto::secretbox::Key {
    let key_data = crypto::hash(
        [
            crypto::hash(accept.message_key().as_ref()).as_ref(),
            receiver_session_key.as_ref(),
        ]
        .concat(),
    );
    crypto::secretbox::Key::from_slice(&key_data).unwrap()
}

#[cfg(test)]
mod test {
    use super::*;

    #[async_std::test]
    async fn run() {
        let (server_writer, server_reader) = async_pipe::pipe();
        let (client_writer, client_reader) = async_pipe::pipe();

        let mut client_stream = duplexify::Duplex::new(server_reader, client_writer);
        let mut server_stream = duplexify::Duplex::new(client_reader, server_writer);

        let network_identifier = [0u8; 32];
        let server_identity = crypto::sign::gen_keypair();
        let server = Server::new(&network_identifier, &server_identity.0, &server_identity.1);

        let client_identity = crypto::sign::gen_keypair();
        let client = Client::new(
            &network_identifier,
            &server_identity.0,
            &client_identity.0,
            &client_identity.1,
        );

        let (client_result, server_result) = futures::join!(
            client.handshake(&mut client_stream),
            server.handshake(&mut server_stream)
        );

        let client_params = client_result.unwrap();
        let server_params = server_result.unwrap();

        assert_eq!(client_params.encrypt, server_params.decrypt);
        assert_eq!(client_params.decrypt, server_params.encrypt);
    }
}
