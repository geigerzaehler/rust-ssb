use chashmap::CHashMap;
use futures::channel::oneshot;
use futures::prelude::*;
use std::pin::Pin;
use std::sync::Arc;

use super::packet::{self, Request, Response};

type RequestSink =
    dyn Sink<Request, Error = Box<dyn std::error::Error + Send + Sync + 'static>> + Send;

/// Client for an application agnostic RPC protocol described in the [Scuttlebutt
/// Protocol Guide][ssb-prot].
///
/// [ssb-prot]: https://ssbc.github.io/scuttlebutt-protocol-guide/#rpc-protocol
pub struct Client {
    sink: Pin<Box<RequestSink>>,
    next_request_number: u32,
    pending_async_requests: Arc<CHashMap<u32, oneshot::Sender<AsyncResponse>>>,
    packet_reader_handle: async_std::task::JoinHandle<()>,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("sink", &"Pin<Box<dyn Sink>>")
            .field("pending_async_requests", &self.pending_async_requests)
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
        let pending_async_requests =
            Arc::new(CHashMap::<u32, oneshot::Sender<AsyncResponse>>::new());
        let pending_async_requests2 = Arc::clone(&pending_async_requests);
        let packet_reader_task = async_std::task::spawn(async move {
            Self::consume_responses(response_stream, &pending_async_requests2).await
        });
        Self {
            sink: Box::pin(request_sink.sink_map_err(|error| {
                Box::new(error) as Box<dyn std::error::Error + Send + Sync + 'static>
            })),
            next_request_number: 1,
            pending_async_requests,
            packet_reader_handle: packet_reader_task,
        }
    }

    pub async fn join(self) {
        self.packet_reader_handle.await
    }

    #[tracing::instrument(skip(response_stream, pending_async_requests))]
    async fn consume_responses<Stream_>(
        response_stream: Stream_,
        pending_async_requests: &CHashMap<u32, oneshot::Sender<AsyncResponse>>,
    ) -> ()
    where
        Stream_: Stream<Item = Response> + Send + Unpin + 'static,
    {
        let mut response_stream = response_stream;
        while let Some(response) = response_stream.next().await {
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
                res => todo!("unhandled response {:?}", res),
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
        let (sender, receiver) = oneshot::channel();
        self.pending_async_requests.insert(request_number, sender);
        self.sink
            .send(request)
            .await
            .map_err(|error| AsyncRequestError::Send { error })?;
        Ok(receiver
            .await
            .expect("Response channel dropped. Possible reuse of request number"))
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
/// Error returned by [Client::send_async].
pub enum AsyncRequestError {
    /// Failed to send the request to the server
    #[error("Failed to send request")]
    Send {
        /// Error returned by the underlying transport channel
        #[source]
        error: Box<dyn std::error::Error + Send + Sync + 'static>,
    },
}
