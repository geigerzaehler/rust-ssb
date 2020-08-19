use chashmap::CHashMap;
use futures::channel::oneshot;
use futures::prelude::*;
use std::sync::Arc;

use super::packet::{self, Packet, RequestType};
use super::receive::PacketStream;
use crate::utils::DynError;

#[derive(Debug)]
pub struct Client<Sink, Stream> {
    sink: Sink,
    next_request_number: u32,
    pending_async_requests: Arc<CHashMap<u32, oneshot::Sender<AsyncResponse>>>,
    packet_reader_task: async_std::task::JoinHandle<()>,
    _stream: std::marker::PhantomData<Stream>,
}

/// A [Client] that erases all type parameters
pub type BoxClient = Client<
    Box<dyn Sink<Vec<u8>, Error = DynError> + Unpin>,
    Box<dyn Stream<Item = Result<Vec<u8>, DynError>> + Unpin + Send + 'static>,
>;

impl<Sink_, Stream_, StreamError_> Client<Sink_, Stream_>
where
    Sink_: Sink<Vec<u8>> + Unpin + 'static,
    Stream_: Stream<Item = Result<Vec<u8>, StreamError_>> + Unpin + Send + 'static,
    StreamError_: std::error::Error + Send + Sync + 'static,
    Sink_::Error: std::error::Error + Send + Sync + 'static,
{
    pub fn new(sink: Sink_, stream: Stream_) -> Self {
        let pending_async_requests =
            Arc::new(CHashMap::<u32, oneshot::Sender<AsyncResponse>>::new());
        let pending_async_requests2 = Arc::clone(&pending_async_requests);
        let packet_reader_task = async_std::task::spawn(async move {
            let packet_stream = PacketStream::new(stream);
            Self::consume_packets(packet_stream, &pending_async_requests2).await;
        });
        Self {
            sink,
            next_request_number: 1,
            pending_async_requests,
            packet_reader_task,
            _stream: std::marker::PhantomData,
        }
    }

    async fn consume_packets(
        mut packet_stream: PacketStream<Stream_>,
        pending_async_requests: &CHashMap<u32, oneshot::Sender<AsyncResponse>>,
    ) {
        loop {
            let packet = packet_stream.next().await;
            let packet = match packet {
                // TODO handle error
                Some(packet) => packet.unwrap(),
                None => break,
            };
            match packet {
                Packet::AsyncResponse { number, body } => {
                    pending_async_requests.alter(number, |opt_respond| {
                        if let Some(respond) = opt_respond {
                            let _result = respond.send(AsyncResponse::from(body));
                        }
                        None
                    })
                }
                Packet::AsyncErrorResponse {
                    number,
                    name,
                    message,
                } => pending_async_requests.alter(number, |opt_respond| {
                    if let Some(respond) = opt_respond {
                        let _result = respond.send(AsyncResponse::Error { name, message });
                    }
                    None
                }),
                Packet::Request { .. } => tracing::warn!(msg = "ingoring rpc request"),
            }
        }
    }

    // TODO underlying protocol error
    pub async fn send_async(
        &mut self,
        method: Vec<String>,
        args: Vec<serde_json::Value>,
    ) -> Result<AsyncResponse, AsyncRequestError> {
        let request_number = self.next_request_number;
        self.next_request_number += 1;

        let packet = Packet::Request {
            number: request_number,
            typ: RequestType::Async,
            method,
            args,
        };
        let (sender, receiver) = oneshot::channel();
        self.pending_async_requests.insert(request_number, sender);
        self.sink
            .send(packet.build())
            .await
            .map_err(|error| AsyncRequestError::Send {
                error: DynError::new(error),
            })?;
        Ok(receiver
            .await
            .expect("Response channel dropped. Possible reuse of request number"))
    }

    pub fn boxed(self) -> BoxClient {
        let Self {
            sink,
            next_request_number,
            pending_async_requests,
            packet_reader_task,
            _stream,
        } = self;
        Client {
            sink: Box::new(sink.sink_map_err(DynError::new)),
            next_request_number,
            pending_async_requests,
            packet_reader_task,
            _stream: std::marker::PhantomData,
        }
    }
}

/// Response for [Client::send_async]
#[derive(Clone)]
pub enum AsyncResponse {
    Json(Vec<u8>),
    Blob(Vec<u8>),
    String(String),
    Error { name: String, message: String },
}

impl std::fmt::Debug for AsyncResponse {
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Blob(data) => fmt.debug_tuple("Blob").field(data).finish(),
            Self::String(string) => fmt.debug_tuple("String").field(string).finish(),
            Self::Json(data) => fmt
                .debug_tuple("Json")
                .field(&String::from_utf8_lossy(data))
                .finish(),
            Self::Error { name, message } => fmt
                .debug_struct("Error")
                .field("name", name)
                .field("message", message)
                .finish(),
        }
    }
}

impl From<packet::Body> for AsyncResponse {
    fn from(body: packet::Body) -> Self {
        match body {
            packet::Body::Json(data) => Self::Json(data),
            packet::Body::Blob(data) => Self::Blob(data),
            packet::Body::String(data) => Self::String(data),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AsyncRequestError {
    #[error("Failed to send request")]
    Send {
        // TODO make this generic
        #[source]
        error: DynError,
    },
}
