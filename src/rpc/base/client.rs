use chashmap::CHashMap;
use futures::prelude::*;
use std::pin::Pin;
use std::sync::Arc;

use super::error::Error;
use super::packet::{Body, Request, Response};
use super::stream_message::StreamMessage;
use super::stream_request::{RequestType, StreamRequest};

/// Client for an application agnostic RPC protocol described in the [Scuttlebutt
/// Protocol Guide][ssb-prot].
///
/// [ssb-prot]: https://ssbc.github.io/scuttlebutt-protocol-guide/#rpc-protocol
pub struct Client {
    sink: SharedRequestSink,
    next_request_number: u32,
    pending_async_requests: Arc<CHashMap<u32, futures::channel::oneshot::Sender<AsyncResponse>>>,
    streams: Arc<CHashMap<u32, futures::channel::mpsc::UnboundedSender<Result<Body, Error>>>>,
    packet_reader_handle: async_std::task::JoinHandle<()>,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("sink", &"Pin<Box<dyn Sink>>")
            .field("next_request_number", &self.next_request_number)
            .field("pending_async_requests", &self.pending_async_requests)
            .field("streams", &"Arc<CHashMap<_, _>>")
            .field("packet_reader_task", &self.packet_reader_handle)
            .finish()
    }
}

impl Client {
    pub fn new<RequestSink, Stream_>(request_sink: RequestSink, response_stream: Stream_) -> Self
    where
        RequestSink: Sink<Request> + Send + Unpin + 'static,
        RequestSink::Error: std::error::Error + Send + Sync + 'static,
        Stream_: Stream<Item = Response> + Send + Unpin + 'static,
    {
        let pending_async_requests = Arc::new(CHashMap::new());
        let streams = Arc::new(CHashMap::new());
        let streams2 = Arc::clone(&streams);
        let pending_async_requests2 = Arc::clone(&pending_async_requests);
        let packet_reader_task = async_std::task::spawn(async move {
            Self::consume_responses(response_stream, &pending_async_requests2, &streams2).await
        });
        Self {
            sink: SharedRequestSink::new(request_sink),
            next_request_number: 1,
            pending_async_requests,
            streams,
            packet_reader_handle: packet_reader_task,
        }
    }

    pub async fn join(self) {
        self.packet_reader_handle.await
    }

    #[tracing::instrument(skip(response_stream, pending_async_requests, streams))]
    async fn consume_responses<Stream_>(
        response_stream: Stream_,
        pending_async_requests: &CHashMap<u32, futures::channel::oneshot::Sender<AsyncResponse>>,
        streams: &CHashMap<u32, futures::channel::mpsc::UnboundedSender<Result<Body, Error>>>,
    ) -> ()
    where
        Stream_: Stream<Item = Response> + Send + Unpin + 'static,
    {
        let mut response_stream = response_stream;
        while let Some(response) = response_stream.next().await {
            tracing::trace!(?response, "received response");
            match response {
                Response::AsyncOk { number, body } => {
                    pending_async_requests.alter(number, |opt_respond| {
                        if let Some(respond) = opt_respond {
                            // TODO handle error
                            respond.send(AsyncResponse::from(body)).unwrap();
                        } else {
                            tracing::error!(number, ?body, "no matching response");
                        }
                        None
                    })
                }
                Response::AsyncErr {
                    number,
                    name,
                    message,
                } => {
                    pending_async_requests.alter(number, |opt_respond| {
                        if let Some(respond) = opt_respond {
                            // TODO handle error
                            respond
                                .send(AsyncResponse::Error { name, message })
                                .unwrap();
                        } else {
                            todo!("no response listener for error")
                        }
                        None
                    })
                }
                Response::Stream { number, message } => match message {
                    StreamMessage::Data(body) => {
                        if let Some(stream) = streams.get_mut(&number) {
                            // We don’t care if the client user drops the source.
                            let _ = stream.unbounded_send(Ok(body));
                        } else {
                            tracing::warn!(stream_id = ?number, "received response for unknown stream");
                        }
                    }
                    StreamMessage::Error(error) => {
                        if let Some(stream) = streams.remove(&number) {
                            // We don’t care if the client user drops the source.
                            let _ = stream.unbounded_send(Err(error));
                        } else {
                            tracing::warn!(stream_id = ?number, "received response for unknown stream");
                        }
                    }
                    StreamMessage::End => {
                        if streams.remove(&number).is_none() {
                            tracing::warn!(stream_id = ?number, "received response for unknown stream");
                        }
                    }
                },
            }
        }
    }

    /// Send a `async` type request to the server and return the response.
    pub async fn send_async(
        &mut self,
        method: Vec<String>,
        args: Vec<serde_json::Value>,
    ) -> Result<AsyncResponse, AsyncRequestError> {
        let request_number = self.next_request_number;
        self.next_request_number += 1;

        let request = Request::Async {
            number: request_number,
            method,
            args,
        };
        let (sender, receiver) = futures::channel::oneshot::channel();
        self.pending_async_requests.insert(request_number, sender);
        self.sink
            .send(request)
            .await
            .map_err(|error| AsyncRequestError::Send { error })?;
        Ok(receiver
            .await
            .expect("Response channel dropped. Possible reuse of request number"))
    }

    pub async fn start_duplex(
        &mut self,
        method: Vec<String>,
        args: Vec<serde_json::Value>,
    ) -> (BoxStreamSource, BoxStreamSink) {
        self.start_stream(RequestType::Duplex, method, args).await
    }

    pub async fn start_stream(
        &mut self,
        type_: RequestType,
        method: Vec<String>,
        args: Vec<serde_json::Value>,
    ) -> (BoxStreamSource, StreamRequestSender) {
        let request_number = self.next_request_number;
        self.next_request_number += 1;

        self.sink
            .send(
                StreamRequest {
                    name: method,
                    type_,
                    args,
                }
                .into_request(request_number),
            )
            .await
            .unwrap();

        let (peer_items_sender, peer_items_receiver) = futures::channel::mpsc::unbounded();
        self.streams.insert(request_number, peer_items_sender);
        let stream_request_sender = self.sink.stream(request_number);
        (Box::pin(peer_items_receiver), stream_request_sender)
    }
}

pub(super) type BoxStreamSource = futures::stream::BoxStream<'static, Result<Body, Error>>;

pub(super) type BoxStreamSink = StreamRequestSender;

type BoxRequestSink = Pin<Box<dyn Sink<Request, Error = anyhow::Error> + Send>>;

/// Clonable [Sink] for [Request]s.
#[derive(Debug, Clone)]
struct SharedRequestSink {
    sink: Arc<futures::lock::Mutex<BoxRequestSink>>,
}

impl SharedRequestSink {
    pub fn new<RequestSink>(request_sink: RequestSink) -> Self
    where
        RequestSink: Sink<Request> + Send + 'static,
        RequestSink::Error: std::error::Error + Send + Sync + 'static,
    {
        Self {
            sink: Arc::new(futures::lock::Mutex::new(Box::pin(
                request_sink.sink_map_err(anyhow::Error::from),
            ))),
        }
    }

    fn stream(&self, id: u32) -> StreamRequestSender {
        StreamRequestSender {
            request_sender: self.clone(),
            id,
        }
    }
    async fn send(&self, request: Request) -> anyhow::Result<()> {
        let mut inner = self.sink.lock().await;
        inner.send(request).await?;
        Ok(())
    }

    async fn send_stream_message(
        &self,
        stream_id: u32,
        stream_message: StreamMessage,
    ) -> anyhow::Result<()> {
        self.send(stream_message.into_request(stream_id)).await
    }
}

#[derive(Debug)]
pub struct StreamRequestSender {
    request_sender: SharedRequestSink,
    id: u32,
}

impl StreamRequestSender {
    pub async fn send(&self, data: Body) -> anyhow::Result<()> {
        self.request_sender
            .send_stream_message(self.id, StreamMessage::Data(data))
            .await
    }

    pub async fn close(self) -> anyhow::Result<()> {
        self.request_sender
            .send_stream_message(self.id, StreamMessage::End)
            .await
    }

    pub async fn error(self, error: Error) -> anyhow::Result<()> {
        self.request_sender
            .send_stream_message(self.id, StreamMessage::Error(error))
            .await
    }
}

/// Response returned by [Client::send_async].
#[derive(Clone, PartialEq, Eq)]
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

impl From<Body> for AsyncResponse {
    fn from(body: Body) -> Self {
        match body {
            Body::Json(data) => Self::Json(data),
            Body::Blob(data) => Self::Blob(data),
            Body::String(data) => Self::String(data),
        }
    }
}

#[derive(Debug, thiserror::Error)]
/// Error returned by [Client::send_async].
pub enum AsyncRequestError {
    /// Failed to send the request to the server
    #[error("Failed to send request")]
    Send {
        /// Error returned by the underlying transport channel
        #[source]
        error: anyhow::Error,
    },
}
