use futures::future::BoxFuture;
use futures::prelude::*;
use futures::stream::BoxStream;
use std::collections::HashMap;
use std::{pin::Pin, task::Poll};

use super::packet::Response;

pub use super::packet::Body;
pub use super::{Error, StreamMessage};

#[derive(Debug, Clone)]
pub enum AsyncResponse {
    Ok(Body),
    Err(Error),
}

impl AsyncResponse {
    pub fn json_ok(value: &impl serde::Serialize) -> Self {
        Self::Ok(Body::json(value))
    }

    pub(super) fn into_response(self, number: u32) -> Response {
        match self {
            AsyncResponse::Ok(body) => Response::AsyncOk { number, body },
            AsyncResponse::Err(Error { name, message }) => Response::AsyncErr {
                number,
                name,
                message,
            },
        }
    }
}

#[derive(Debug)]
pub enum SinkError {
    Done,
    Error(Error),
}

/// Error for duplex stream sinks that indicates that the sink has been closed and will not process
/// any more messages.
#[derive(Debug)]
pub struct SinkClosed;

#[derive(Default)]
pub struct Service {
    async_handlers: HashMap<Vec<String>, Handler<BoxFuture<'static, AsyncResponse>>>,
    stream_handlers: HashMap<Vec<String>, Handler<(BoxEndpointStream, BoxEndpointSink)>>,
}

impl Service {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn add_async<Args, Fut>(
        &mut self,
        method: impl ToString,
        f: impl Fn(Args) -> Fut + Send + 'static,
    ) where
        Args: serde::de::DeserializeOwned,
        Fut: Future<Output = AsyncResponse> + Send + 'static,
    {
        self.async_handlers.insert(
            vec![method.to_string()],
            Box::new(move |args| {
                let args = serde_json::Value::Array(args);
                match serde_json::from_value::<Args>(args) {
                    Ok(args) => f(args).boxed(),
                    Err(error) => futures::future::ready(AsyncResponse::Err(
                        deserialize_arguments_error(error),
                    ))
                    .boxed(),
                }
            }),
        );
    }

    pub fn add_source<Args, Source>(
        &mut self,
        method: impl ToString,
        f: impl Fn(Args) -> Source + Send + 'static,
    ) where
        Args: serde::de::DeserializeOwned,
        Source: Stream<Item = Result<Body, Error>> + Send + 'static,
    {
        self.stream_handlers.insert(
            vec![method.to_string()],
            Box::new(move |args| {
                let args = serde_json::Value::Array(args);
                match serde_json::from_value::<Args>(args) {
                    Ok(args) => stream_to_endpoint(f(args)),
                    Err(error) => error_endpoint(deserialize_arguments_error(error)),
                }
            }),
        );
    }

    pub fn add_sink<Args, Sink_>(
        &mut self,
        method: impl ToString,
        f: impl Fn(Args) -> Sink_ + Send + 'static,
    ) where
        Args: serde::de::DeserializeOwned,
        Sink_: Sink<StreamMessage, Error = SinkError> + Send + 'static,
    {
        self.stream_handlers.insert(
            vec![method.to_string()],
            Box::new(move |args| {
                let args = serde_json::Value::Array(args);
                match serde_json::from_value::<Args>(args) {
                    Ok(args) => sink_to_endpoint(f(args)),
                    Err(error) => error_endpoint(deserialize_arguments_error(error)),
                }
            }),
        );
    }

    pub fn add_duplex<Args, Source, Sink_>(
        &mut self,
        method: impl ToString,
        f: impl Fn(Args) -> (Source, Sink_) + Send + 'static,
    ) where
        Args: serde::de::DeserializeOwned,
        Source: Stream<Item = Result<Body, Error>> + Send + 'static,
        Sink_: Sink<StreamMessage, Error = SinkClosed> + Send + 'static,
    {
        let method2 = method.to_string();
        self.stream_handlers.insert(
            vec![method.to_string()],
            Box::new(move |args| {
                let args = serde_json::Value::Array(args);
                match serde_json::from_value::<Args>(args) {
                    Ok(args) => {
                        let (source, sink) = f(args);
                        (source.boxed(), Box::pin(sink))
                    }
                    Err(error) => {
                        tracing::warn!(method = ?method2, ?error, "failed to deserialize arguments");
                        error_endpoint(deserialize_arguments_error(error))
                    }
                }
            }),
        );
    }

    pub fn add_service(&mut self, group: impl ToString, service: Self) {
        let Self {
            async_handlers,
            stream_handlers,
        } = service;
        self.async_handlers
            .extend(async_handlers.into_iter().map(|(mut k, v)| {
                k.insert(0, group.to_string());
                (k, v)
            }));
        self.stream_handlers
            .extend(stream_handlers.into_iter().map(|(mut k, v)| {
                k.insert(0, group.to_string());
                (k, v)
            }));
    }

    pub(super) fn handle_async(
        &self,
        method: Vec<String>,
        args: Vec<serde_json::Value>,
    ) -> BoxFuture<'static, AsyncResponse> {
        match self.async_handlers.get(&method) {
            Some(handler) => handler(args),
            None => {
                tracing::warn!(method = ?method.join(","), "missing async method");
                futures::future::ready(AsyncResponse::Err(method_not_found_error(&method))).boxed()
            }
        }
    }

    pub(super) fn handle_stream(
        &self,
        method: Vec<String>,
        args: Vec<serde_json::Value>,
    ) -> (BoxEndpointStream, BoxEndpointSink) {
        match self.stream_handlers.get(&method) {
            Some(handler) => handler(args),
            None => {
                tracing::warn!(method = ?method.join("."), "missing stream method");
                error_endpoint(method_not_found_error(&method))
            }
        }
    }
}

impl std::fmt::Debug for Service {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Service")
            .field(
                "async_handlers",
                &self.async_handlers.keys().collect::<Vec<_>>(),
            )
            .field(
                "stream_handlers",
                &self.stream_handlers.keys().collect::<Vec<_>>(),
            )
            .finish()
    }
}

pub(super) type BoxEndpointStream = BoxStream<'static, Result<Body, Error>>;

pub(super) type BoxEndpointSink = Pin<Box<dyn Sink<StreamMessage, Error = SinkClosed> + Send>>;

type Handler<T> = Box<dyn Fn(Vec<serde_json::Value>) -> T + Send + 'static>;

fn error_endpoint(error: Error) -> (BoxEndpointStream, BoxEndpointSink) {
    let sink = futures::sink::drain().sink_map_err(|infallible| match infallible {});
    let source = futures::stream::once(async move { Err(error) });
    (source.boxed(), Box::pin(sink))
}

fn sink_to_endpoint(
    sink: impl Sink<StreamMessage, Error = SinkError> + Send + 'static,
) -> (BoxEndpointStream, BoxEndpointSink) {
    let (response_sender, response_receiver) =
        futures::channel::oneshot::channel::<Result<Body, Error>>();
    let source = crate::utils::OneshotStream::new(response_receiver);
    let duplex_sink = sink
        .with::<_, _, _, SinkError>(|stream_message| {
            futures::future::ready({
                match stream_message {
                    StreamMessage::Data(_) => Ok(stream_message),
                    StreamMessage::Error(err) => Err(SinkError::Error(err)),
                    StreamMessage::End => Err(SinkError::Done),
                }
            })
        })
        .sink_map_err(|err| {
            match err {
                SinkError::Done => drop(response_sender),
                SinkError::Error(err) => response_sender.send(Err(err)).unwrap(),
            }
            SinkClosed
        });
    (source.boxed(), Box::pin(duplex_sink))
}

fn stream_to_endpoint(
    source: impl Stream<Item = Result<Body, Error>> + Send + 'static,
) -> (BoxEndpointStream, BoxEndpointSink) {
    let mut source = Box::pin(source);
    let mut done = false;
    let (peer_end_sender, mut peer_end_receiver) = futures::channel::oneshot::channel();

    let duplex_source = futures::stream::poll_fn(move |ctx| -> Poll<Option<Result<Body, Error>>> {
        if done {
            return Poll::Ready(None);
        }
        match peer_end_receiver.poll_unpin(ctx) {
            Poll::Ready(value) => {
                done = true;
                let response = match value {
                    Ok(stream_message) => match stream_message {
                        StreamMessage::Data(_) => Some(Err(Error {
                            name: "SENT_DATA_TO_SOURCE".to_string(),
                            message: "Cannot send data to a \"source\" stream".to_string(),
                        })),
                        StreamMessage::Error(error) => Some(Err(error)),
                        StreamMessage::End => None,
                    },
                    Err(_cancelled) => None,
                };
                return Poll::Ready(response);
            }
            Poll::Pending => {}
        }
        source.as_mut().poll_next(ctx)
    });
    (
        duplex_source.boxed(),
        Box::pin(crate::utils::OneshotSink::new(peer_end_sender).sink_map_err(|_| SinkClosed)),
    )
}

fn method_not_found_error(method: &[String]) -> Error {
    let name = "METHOD_NOT_FOUND".to_string();
    let message = format!("Method \"{}\" not found", method.join("."));
    Error { name, message }
}

fn deserialize_arguments_error(error: serde_json::Error) -> Error {
    Error {
        name: "ArgumentError".to_string(),
        message: format!("Failed to deserialize arguments {}", error),
    }
}
