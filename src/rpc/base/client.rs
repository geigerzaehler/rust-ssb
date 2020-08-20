use chashmap::CHashMap;
use futures::channel::oneshot;
use futures::prelude::*;
use std::pin::Pin;
use std::sync::Arc;

use super::packet::{self, Packet, RequestType};
use super::packet_stream::PacketStream;

type ClientSink = dyn Sink<Vec<u8>, Error = Box<dyn std::error::Error + Send + Sync + 'static>>;

pub struct Client {
    sink: Pin<Box<ClientSink>>,
    next_request_number: u32,
    pending_async_requests: Arc<CHashMap<u32, oneshot::Sender<AsyncResponse>>>,
    packet_reader_task: async_std::task::JoinHandle<()>,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("sink", &"Pin<Box<dyn Sink>>")
            .field("pending_async_requests", &self.pending_async_requests)
            .field("packet_reader_task", &self.packet_reader_task)
            .finish()
    }
}

impl Client {
    pub fn new<Sink_, Stream_>(sink: Sink_, stream: Stream_) -> Self
    where
        Sink_: Sink<Vec<u8>> + Unpin + 'static,
        Sink_::Error: std::error::Error + Send + Sync + 'static,
        Stream_: TryStream<Ok = Vec<u8>> + Send + Unpin + 'static,
        Stream_::Error: std::error::Error + 'static,
    {
        let pending_async_requests =
            Arc::new(CHashMap::<u32, oneshot::Sender<AsyncResponse>>::new());
        let pending_async_requests2 = Arc::clone(&pending_async_requests);
        let packet_reader_task = async_std::task::spawn(async move {
            Self::consume_packets(stream, &pending_async_requests2).await;
        });
        Self {
            sink: Box::pin(sink.sink_map_err(|error| {
                Box::new(error) as Box<dyn std::error::Error + Send + Sync + 'static>
            })),
            next_request_number: 1,
            pending_async_requests,
            packet_reader_task,
        }
    }

    async fn consume_packets<Stream_>(
        stream: Stream_,
        pending_async_requests: &CHashMap<u32, oneshot::Sender<AsyncResponse>>,
    ) where
        Stream_: TryStream<Ok = Vec<u8>> + Unpin,
        Stream_::Error: std::error::Error + 'static,
    {
        let mut packet_stream = PacketStream::new(stream);
        loop {
            let next_item = packet_stream.try_next().await;
            let packet = match next_item {
                Ok(Some(packet)) => packet,
                // TOOO handle closing
                Ok(None) => break,
                // TOOO handle error
                Err(_err) => todo!(),
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
            .map_err(|error| AsyncRequestError::Send { error })?;
        Ok(receiver
            .await
            .expect("Response channel dropped. Possible reuse of request number"))
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
        #[source]
        error: Box<dyn std::error::Error + Send + Sync + 'static>,
    },
}
