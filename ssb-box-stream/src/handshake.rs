// We allow this to align with the names used in the protocol guide.
#![allow(non_snake_case)]

use futures::prelude::*;

use crate::crypto;

const HELLO_MESSAGE_LEN: usize = 64;
const CLIENT_AUTHENTICATE_MESSAGE_LEN: usize = 112;
const ACCEPT_CIPHER_MESSAGE_LEN: usize = 80;

/// Errors returned when running the handshake protocol.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Failed to read data from remote
    #[error("Failed to read data from remote")]
    ReadFailed(#[source] std::io::Error),
    /// Failed to write data to remote
    #[error("Failed to write data to remote")]
    WriteFailed(#[source] std::io::Error),

    /// Failed to verify `hello` message from remote
    #[error("Failed to verify `hello` message from remote")]
    HelloMessageInvalid,

    /// Failed to decrypt `authenticate` message
    #[error("Failed to decrypt `authenticate` message")]
    AuthenticateMessageDecryptFailed,
    /// Invalid signature in `authenticate` message
    #[error("Invalid signature in `authenticate` message")]
    AuthenticateSignatureInvalid,

    #[error("Server did not accept handshake. Server identity may be wrong")]
    AcceptConnectionClosed,

    /// Failed to decrypt `accept` message
    #[error("Failed to decrypt `accept` message. Client may have used the wrong key")]
    AcceptMessageDecryptFailed,
    /// Invalid signature in `accept` message
    #[error("Invalid signature in `accept` message")]
    AcceptSignatureInvalid,
}

/// Parameters to establish a secure connection as a client
///
/// ```no_run
/// # use ssb_box_stream::*;
/// # use futures::prelude::*;
/// # #[async_std::main]
/// # async fn main () -> Result<(), Box<dyn std::error::Error>> {
/// let network_identifier = [0u8; 32];
/// let server_identity_pk = sodiumoxide::crypto::sign::gen_keypair().0;
/// let client_identity = sodiumoxide::crypto::sign::gen_keypair();
/// let client = Client::new(
///     &network_identifier,
///     &server_identity_pk,
///     &client_identity.0,
///     &client_identity.1,
/// );
///
/// let mut stream = async_std::net::TcpStream::connect("localhost:5555").await.unwrap();
/// let (send, recv) = client.connect(stream).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct Client {
    network_identifier: crypto::auth::Key,
    identity_pk: crypto::sign::PublicKey,
    identity_sk: crypto::sign::SecretKey,
    server_identity_pk: crypto::sign::PublicKey,
}

impl Client {
    pub fn new(
        network_identifier: &[u8; 32],
        server_identity_pk: &sodiumoxide::crypto::sign::PublicKey,
        identity_pk: &sodiumoxide::crypto::sign::PublicKey,
        identity_sk: &sodiumoxide::crypto::sign::SecretKey,
    ) -> Self {
        let network_identifier = crypto::auth::key_from_array(network_identifier);
        Self {
            network_identifier,
            identity_pk: *identity_pk,
            identity_sk: identity_sk.clone(),
            server_identity_pk: *server_identity_pk,
        }
    }

    /// Execute the handshake protocol for the client and return the encrypted connection.
    pub async fn connect<Stream: AsyncWrite + AsyncRead + Unpin>(
        &self,
        mut stream: Stream,
    ) -> Result<
        (
            crate::Encrypt<futures::io::WriteHalf<Stream>>,
            crate::Decrypt<futures::io::ReadHalf<Stream>>,
        ),
        Error,
    > {
        let params = self.handshake(&mut stream).await?;
        Ok(crate::box_stream(stream, params))
    }

    async fn handshake(
        &self,
        mut stream: impl AsyncRead + AsyncWrite + Unpin,
    ) -> Result<crate::BoxStreamParams, Error> {
        let (session_pk, session_sk) = crypto::box_::gen_keypair();
        let endpoint = Endpoint {
            identity_pk: self.identity_pk,
            identity_sk: self.identity_sk.clone(),
            session_pk,
            session_sk,
            network_identifier: self.network_identifier.clone(),
        };
        stream
            .write_all(&endpoint.hello_message())
            .await
            .map_err(Error::WriteFailed)?;

        let hello_msg = read_hello_bytes(&mut stream).await?;
        let server_session_pk = endpoint.hello_verify(hello_msg)?;
        let authenticate =
            Authenticate::for_client(&endpoint, &self.server_identity_pk, &server_session_pk);

        let msg = authenticate_message(&endpoint, &self.server_identity_pk, &authenticate);
        stream.write_all(&msg).await.map_err(Error::WriteFailed)?;

        let accept = Accept::for_client(&endpoint, &self.server_identity_pk, authenticate);
        let mut reply = [0u8; 80];
        stream.read_exact(&mut reply).await.map_err(|error| {
            if error.kind() == std::io::ErrorKind::UnexpectedEof {
                Error::AcceptConnectionClosed
            } else {
                Error::ReadFailed(error)
            }
        })?;
        accept_message_verify(&self, &accept, reply)?;

        Ok(box_stream_params(
            &endpoint,
            &accept,
            &self.server_identity_pk,
            &server_session_pk,
        ))
    }
}

/// Parameters to establish a secure connection as a server
///
/// ```no_run
/// # use ssb_box_stream::*;
/// # use futures::prelude::*;
/// # #[async_std::main]
/// # async fn main () -> Result<(), Box<dyn std::error::Error>> {
/// let network_identifier = [0u8; 32];
/// let server_identity = sodiumoxide::crypto::sign::gen_keypair();
/// let server = Server::new(
///     &network_identifier,
///     &server_identity.0,
///     &server_identity.1,
/// );
///
/// let mut listener = async_std::net::TcpListener::bind("localhost:5555").await.unwrap();
/// let (stream, _) = listener.accept().await?;
///
/// let (send, recv, client_key) = server.accept(stream).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct Server {
    network_identifier: crypto::auth::Key,
    identity_pk: crypto::sign::PublicKey,
    identity_sk: crypto::sign::SecretKey,
}

impl Server {
    pub fn new(
        network_identifier: &[u8; 32],
        identity_pk: &sodiumoxide::crypto::sign::PublicKey,
        identity_sk: &sodiumoxide::crypto::sign::SecretKey,
    ) -> Self {
        let network_identifier = crypto::auth::key_from_array(network_identifier);
        Self {
            network_identifier,
            identity_pk: *identity_pk,
            identity_sk: identity_sk.clone(),
        }
    }

    /// Execute the handshake protocol for the server and return the encrypted connection
    /// and the clients public identity key
    pub async fn accept<Stream: AsyncRead + AsyncWrite + Unpin>(
        &self,
        stream: Stream,
    ) -> Result<
        (
            crate::Encrypt<futures::io::WriteHalf<Stream>>,
            crate::Decrypt<futures::io::ReadHalf<Stream>>,
            crypto::sign::PublicKey,
        ),
        Error,
    > {
        let mut stream = stream;
        let (params, client_identity_pk) = self.handshake(&mut stream).await?;
        let (sink, stream) = crate::box_stream(stream, params);
        Ok((sink, stream, client_identity_pk))
    }

    /// Execute the handshake protocol for the server and return the box stream
    /// parameters and the clients public identity key
    async fn handshake(
        &self,
        mut stream: impl AsyncRead + AsyncWrite + Unpin,
    ) -> Result<(crate::BoxStreamParams, crypto::sign::PublicKey), Error> {
        let (session_pk, session_sk) = crypto::box_::gen_keypair();
        let endpoint = Endpoint {
            identity_pk: self.identity_pk,
            identity_sk: self.identity_sk.clone(),
            session_pk,
            session_sk,
            network_identifier: self.network_identifier.clone(),
        };

        let hello_msg = read_hello_bytes(&mut stream).await?;
        let client_session_pk = endpoint.hello_verify(hello_msg)?;
        let authenticate = Authenticate::for_server(&endpoint, &client_session_pk);

        stream
            .write_all(&endpoint.hello_message())
            .await
            .map_err(Error::WriteFailed)?;

        let mut authenticate_msg = [0u8; CLIENT_AUTHENTICATE_MESSAGE_LEN];
        stream
            .read_exact(&mut authenticate_msg)
            .await
            .map_err(Error::ReadFailed)?;

        let accept = authenticate.verify_and_accept(&endpoint, &authenticate_msg)?;

        let accept_message = accept_message(&endpoint, &accept);
        stream
            .write_all(&accept_message)
            .await
            .map_err(Error::WriteFailed)?;

        Ok((
            box_stream_params(
                &endpoint,
                &accept,
                &accept.client_identity_pk,
                &client_session_pk,
            ),
            accept.client_identity_pk,
        ))
    }
}

async fn read_hello_bytes(
    mut stream: impl AsyncRead + Unpin,
) -> Result<[u8; HELLO_MESSAGE_LEN], Error> {
    let mut reply = [0u8; HELLO_MESSAGE_LEN];
    stream
        .read_exact(&mut reply)
        .await
        .map_err(Error::ReadFailed)?;
    Ok(reply)
}

fn authenticate_message(
    client: &Endpoint,
    server_identity_pk: &crypto::sign::PublicKey,
    authenticate: &Authenticate,
) -> Vec<u8> {
    let msg = authenticate.signature_payload(server_identity_pk);
    let detached_signature_A = crypto::sign::sign_detached(&msg, &client.identity_sk);

    let key = authenticate.message_key();
    let msg = [detached_signature_A.as_ref(), client.identity_pk.as_ref()].concat();
    crypto::secretbox::seal(&msg, &zero_nonce(), &key)
}

fn accept_message(server: &Endpoint, shared_secrets: &Accept) -> Vec<u8> {
    let msg = shared_secrets.signature_payload();
    let detached_signature_B = crypto::sign::sign_detached(&msg, &server.identity_sk);

    crypto::secretbox::seal(
        detached_signature_B.as_ref(),
        &zero_nonce(),
        &shared_secrets.message_key(),
    )
}

fn accept_message_verify(
    client: &Client,
    accept: &Accept,
    cipher_msg: [u8; ACCEPT_CIPHER_MESSAGE_LEN],
) -> Result<(), Error> {
    let detached_signature_B_payload =
        crypto::secretbox::open(&cipher_msg, &zero_nonce(), &accept.message_key())
            .map_err(|()| Error::AcceptMessageDecryptFailed)?;
    let detached_signature_B =
        crypto::sign::Signature::from_slice(&detached_signature_B_payload).unwrap();

    let msg = accept.signature_payload();
    if crypto::sign::verify_detached(&detached_signature_B, &msg, &client.server_identity_pk) {
        Ok(())
    } else {
        Err(Error::AcceptSignatureInvalid)
    }
}

/// Data that identifies an endpoint (client or server) at the start of the handshake.
#[derive(Debug, Clone)]
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

    fn hello_verify(&self, msg: [u8; HELLO_MESSAGE_LEN]) -> Result<crypto::box_::PublicKey, Error> {
        let (tag, payload) = msg.split_at(crypto::auth::TAGBYTES);
        let tag = crypto::auth::Tag::from_slice(tag).unwrap();

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
    fn for_client(
        client: &Endpoint,
        server_identity_pk: &crypto::sign::PublicKey,
        server_session_pk: &crypto::box_::PublicKey,
    ) -> Self {
        let ab = crypto::share_key(&server_session_pk, &client.session_sk).unwrap();

        let aB = crypto::share_key(
            &crypto::sign_to_box_pk(server_identity_pk).unwrap(),
            &client.session_sk,
        )
        .unwrap();

        Self {
            ab,
            aB,
            network_identifier: client.network_identifier.clone(),
            server_session_pk: *server_session_pk,
        }
    }
    fn for_server(server: &Endpoint, client_session_pk: &crypto::box_::PublicKey) -> Self {
        let ab = crypto::share_key(client_session_pk, &server.session_sk).unwrap();

        let aB = crypto::share_key(
            &client_session_pk,
            &crypto::sign_to_box_sk(&server.identity_sk).unwrap(),
        )
        .unwrap();

        Self {
            ab,
            aB,
            network_identifier: server.network_identifier.clone(),
            server_session_pk: server.session_pk,
        }
    }

    /// Verifies a clients `authenticate` messages and return the [Accept] data.
    fn verify_and_accept(
        self,
        server: &Endpoint,
        client_authenticate_msg: &[u8; CLIENT_AUTHENTICATE_MESSAGE_LEN],
    ) -> Result<Accept, Error> {
        let key = self.message_key();
        let msg = crypto::secretbox::open(&client_authenticate_msg[..], &zero_nonce(), &key)
            .map_err(|()| Error::AuthenticateMessageDecryptFailed)?;
        let (detached_signature_A, client_identity_pk) = msg.split_at(crypto::sign::SIGNATUREBYTES);
        let detached_signature_A =
            crypto::sign::Signature::from_slice(detached_signature_A).unwrap();
        let client_identity_pk = crypto::sign::PublicKey::from_slice(client_identity_pk).unwrap();
        let signature_payload = self.signature_payload(&server.identity_pk);
        if crypto::sign::verify_detached(
            &detached_signature_A,
            &signature_payload,
            &client_identity_pk,
        ) {
            let accept =
                Accept::for_server(server, self, &client_identity_pk, &detached_signature_A);
            Ok(accept)
        } else {
            Err(Error::AuthenticateSignatureInvalid)
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
        crypto::secretbox::key_from_array(&key_data)
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
    fn for_client(
        client: &Endpoint,
        server_identity_pk: &crypto::sign::PublicKey,
        authenticate: Authenticate,
    ) -> Self {
        let Ab = crypto::share_key(
            &authenticate.server_session_pk,
            &crypto::sign_to_box_sk(&client.identity_sk).unwrap(),
        )
        .unwrap();

        let msg = [
            client.network_identifier.as_ref(),
            server_identity_pk.as_ref(),
            crypto::hash(&authenticate.ab).as_ref(),
        ]
        .concat();
        let detached_signature_A = crypto::sign::sign_detached(&msg, &client.identity_sk);

        Self {
            authenticate,
            Ab,
            detached_signature_A,
            client_identity_pk: client.identity_pk,
        }
    }

    fn for_server(
        server: &Endpoint,
        authenticate: Authenticate,
        client_identity_pk: &crypto::sign::PublicKey,
        detached_signature_A: &crypto::sign::Signature,
    ) -> Self {
        let Ab = crypto::share_key(
            &crypto::sign_to_box_pk(&client_identity_pk).unwrap(),
            &server.session_sk,
        )
        .unwrap();

        Self {
            authenticate,
            Ab,
            detached_signature_A: *detached_signature_A,
            client_identity_pk: *client_identity_pk,
        }
    }

    /// Returns the key that encrypts the `accept` message of the server.
    fn message_key(&self) -> crypto::secretbox::Key {
        crypto::secretbox::key_from_array(&crypto::hash(
            [
                self.authenticate.network_identifier.as_ref(),
                self.authenticate.ab.as_ref(),
                self.authenticate.aB.as_ref(),
                self.Ab.as_ref(),
            ]
            .concat(),
        ))
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
) -> crate::BoxStreamParams {
    crate::BoxStreamParams {
        send: crate::cipher::Params::new(
            box_stream_key(&accept, remote_identity_pk),
            box_stream_nonce(local, remote_session_pk),
        ),
        receive: crate::cipher::Params::new(
            box_stream_key(&accept, &local.identity_pk),
            box_stream_nonce(local, &local.session_pk),
        ),
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
    crypto::secretbox::key_from_array(&key_data)
}

fn zero_nonce() -> crypto::secretbox::Nonce {
    crypto::secretbox::Nonce::from_slice(&[0u8; 24]).unwrap()
}

#[cfg(test)]
mod test {
    use super::*;

    #[async_std::test]
    async fn run() {
        let _ = sodiumoxide::init();

        let (mut client_stream, mut server_stream) = duplex_pipe();

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
        let (server_params, client_identity_pk) = server_result.unwrap();

        assert_eq!(client_params.send, server_params.receive);
        assert_eq!(client_params.receive, server_params.send);
        assert_eq!(client_identity_pk, client_identity.0);
    }

    #[async_std::test]
    async fn client_with_invalid_server_key() {
        let _ = tracing_subscriber::fmt::try_init();
        let _ = sodiumoxide::init();

        let (mut client_stream, mut server_stream) = duplex_pipe();

        let network_identifier = [0u8; 32];
        let server_identity = crypto::sign::gen_keypair();
        let server = Server::new(&network_identifier, &server_identity.0, &server_identity.1);

        let client_identity = crypto::sign::gen_keypair();
        let (invalid_server_pk, _) = crypto::sign::gen_keypair();
        let client = Client::new(
            &network_identifier,
            &invalid_server_pk,
            &client_identity.0,
            &client_identity.1,
        );

        let (client_result, server_result) =
            futures::join!(client.handshake(&mut client_stream), async move {
                let result = server.handshake(&mut server_stream).await;
                server_stream.close().await.unwrap();
                result
            });

        assert!(matches!(client_result, Err(Error::AcceptConnectionClosed)));

        assert!(matches!(
            server_result,
            Err(Error::AuthenticateMessageDecryptFailed)
        ));
    }

    /// Create a pair of connected read-write pipes
    fn duplex_pipe() -> (impl AsyncRead + AsyncWrite, impl AsyncRead + AsyncWrite) {
        let (a_writer, a_reader) = async_std::os::unix::net::UnixStream::pair().unwrap();
        let (b_writer, b_reader) = async_std::os::unix::net::UnixStream::pair().unwrap();

        let a_to_b = duplexify::Duplex::new(b_reader, a_writer);
        let b_to_a = duplexify::Duplex::new(a_reader, b_writer);
        (a_to_b, b_to_a)
    }
}
