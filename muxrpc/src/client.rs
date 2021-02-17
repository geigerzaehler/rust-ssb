use chashmap::CHashMap;
use futures::prelude::*;
use std::pin::Pin;
use std::sync::Arc;

use crate::error::Error;
use crate::packet::{Body, Request, Response};
use crate::stream_message::StreamMessage;
use crate::stream_request::{StreamRequest, StreamRequestType};

/// Client for an application agnostic RPC protocol described in the [Scuttlebutt
/// Protocol Guide][ssb-prot].
///
/// [ssb-prot]: https://ssbc.github.io/scuttlebutt-protocol-guide/#rpc-protocol
pub struct Client {
    request_sink: BoxRequestSink,
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
    pub fn new<RequestSink, ResponseStream>(
        request_sink: RequestSink,
        response_stream: ResponseStream,
    ) -> Self
    where
        RequestSink: Sink<Request> + Send + Clone + Unpin + 'static,
        RequestSink::Error: std::error::Error + Send + Sync + 'static,
        ResponseStream: Stream<Item = Response> + Send + Unpin + 'static,
    {
        let pending_async_requests = Arc::new(CHashMap::new());
        let streams = Arc::new(CHashMap::new());
        let streams2 = Arc::clone(&streams);
        let pending_async_requests2 = Arc::clone(&pending_async_requests);
        let packet_reader_task = async_std::task::spawn(async move {
            Self::consume_responses(response_stream, &pending_async_requests2, &streams2).await
        });
        Self {
            request_sink: Box::pin(request_sink.sink_map_err(anyhow::Error::from)),
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
                                .send(AsyncResponse::Error(Error { name, message }))
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
        self.request_sink
            .send(request)
            .await
            .map_err(|error| AsyncRequestError::Send { error })?;
        Ok(receiver
            .await
            .expect("Response channel dropped. Possible reuse of request number"))
    }

    /// Send a request to the server to start a duplex stream.
    pub async fn start_duplex(
        &mut self,
        method: Vec<String>,
        args: Vec<serde_json::Value>,
    ) -> anyhow::Result<(BoxStreamSource, StreamSink)> {
        self.start_stream(StreamRequestType::Duplex, method, args)
            .await
    }

    async fn start_stream(
        &mut self,
        type_: StreamRequestType,
        method: Vec<String>,
        args: Vec<serde_json::Value>,
    ) -> anyhow::Result<(BoxStreamSource, StreamSink)> {
        let request_number = self.next_request_number;
        self.next_request_number += 1;

        self.request_sink
            .send(
                StreamRequest {
                    name: method,
                    type_,
                    args,
                }
                .into_request(request_number),
            )
            .await?;

        let (received_messages_sender, received_messages_receiver) =
            futures::channel::mpsc::unbounded();
        self.streams
            .insert(request_number, received_messages_sender);
        let stream_sink = StreamSink {
            request_sink: self.request_sink.dup(),
            id: request_number,
        };
        Ok((Box::pin(received_messages_receiver), stream_sink))
    }
}

pub type BoxStreamSource = futures::stream::BoxStream<'static, Result<Body, Error>>;

type BoxRequestSink = Pin<Box<dyn ClonableRequestSink>>;

trait ClonableRequestSink
where
    Self: Sink<Request, Error = anyhow::Error> + Send,
{
    fn dup(&self) -> BoxRequestSink;
}

impl<T> ClonableRequestSink for T
where
    T: Sink<Request, Error = anyhow::Error> + Send,
    T: Clone + 'static,
{
    fn dup(&self) -> BoxRequestSink {
        Box::pin(self.clone())
    }
}

/// Send messages for a specific stream to the peer.
///
/// The sink must be explicitly closed by calling [StreamSink::close] or [StreamSink::error] to
/// tell the server that the client will not send messages anymore. It is _not sufficient_ to drop
/// [StreamSink].
pub struct StreamSink {
    request_sink: BoxRequestSink,
    id: u32,
}

impl std::fmt::Debug for StreamSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamSink")
            .field("request_sink", &"BoxRequestSink")
            .field("id", &self.id)
            .finish()
    }
}
impl StreamSink {
    pub async fn send(&mut self, data: Body) -> anyhow::Result<()> {
        self.send_message(StreamMessage::Data(data)).await
    }

    pub async fn close(mut self) -> anyhow::Result<()> {
        self.send_message(StreamMessage::End).await
    }

    pub async fn error(mut self, error: Error) -> anyhow::Result<()> {
        self.send_message(StreamMessage::Error(error)).await
    }

    async fn send_message(&mut self, stream_message: StreamMessage) -> anyhow::Result<()> {
        self.request_sink
            .send(stream_message.into_request(self.id))
            .await
    }
}

/// Response returned by [Client::send_async].
#[derive(Clone, PartialEq, Eq)]
pub enum AsyncResponse {
    Json(Vec<u8>),
    Blob(Vec<u8>),
    String(String),
    Error(Error),
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
            Self::Error(Error { name, message }) => fmt
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
