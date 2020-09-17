use futures::future::BoxFuture;
use futures::prelude::*;
use futures::stream::BoxStream;
use std::collections::HashMap;
use std::{pin::Pin, task::Poll};

use crate::rpc::base::packet::{Body, Response};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Error {
    pub name: String,
    pub message: String,
}

impl Error {
    pub(super) fn method_not_found(method: &[String]) -> Self {
        let name = "METHOD_NOT_FOUND".to_string();
        let message = format!("Method \"{}\" not found", method.join("."));
        Self { name, message }
    }

    pub(super) fn deserialize_arguments(error: serde_json::Error) -> Self {
        Self {
            name: "ArgumentError".to_string(),
            message: format!("Failed to deserialize arguments {}", error),
        }
    }
}

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
pub enum StreamItem {
    Data(Body),
    Error(Error),
    End,
}

impl StreamItem {
    pub(super) fn into_response(self, number: u32) -> Response {
        match self {
            StreamItem::Data(body) => Response::StreamData { number, body },
            StreamItem::Error(Error { name, message }) => Response::StreamError {
                number,
                name,
                message,
            },
            StreamItem::End => Response::StreamEnd { number },
        }
    }

    pub(super) fn is_end(&self) -> bool {
        match self {
            StreamItem::Data(_) => false,
            StreamItem::Error(_) => true,
            StreamItem::End => true,
        }
    }
}

impl From<Option<Result<Body, Error>>> for StreamItem {
    fn from(item: Option<Result<Body, Error>>) -> Self {
        match item {
            Some(Ok(body)) => Self::Data(body),
            Some(Err(error)) => Self::Error(error),
            None => Self::End,
        }
    }
}

#[derive(Debug)]
pub enum SinkError {
    Done,
    Error(Error),
}

pub(super) type BoxEndpointStream = BoxStream<'static, Result<Body, Error>>;

pub(super) type BoxEndpointSink = Pin<Box<dyn Sink<StreamItem, Error = ()> + Send>>;

pub(super) type BoxEndpoint = (BoxEndpointStream, BoxEndpointSink);

type Handler<T> = Box<dyn Fn(Vec<serde_json::Value>) -> T + Send + 'static>;

#[derive(Default)]
pub struct Service {
    async_handlers: HashMap<Vec<String>, Handler<BoxFuture<'static, AsyncResponse>>>,
    stream_handlers: HashMap<Vec<String>, Handler<BoxEndpoint>>,
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
                        Error::deserialize_arguments(error),
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
                    Err(error) => error_endpoint(Error::deserialize_arguments(error)),
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
        Sink_: Sink<StreamItem, Error = SinkError> + Send + 'static,
    {
        self.stream_handlers.insert(
            vec![method.to_string()],
            Box::new(move |args| {
                let args = serde_json::Value::Array(args);
                match serde_json::from_value::<Args>(args) {
                    Ok(args) => sink_to_endpoint(f(args)),
                    Err(error) => error_endpoint(Error::deserialize_arguments(error)),
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
        Sink_: Sink<StreamItem, Error = ()> + Send + 'static,
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
                        error_endpoint(Error::deserialize_arguments(error))
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
                futures::future::ready(AsyncResponse::Err(Error::method_not_found(&method))).boxed()
            }
        }
    }

    pub(super) fn handle_stream(
        &self,
        method: Vec<String>,
        args: Vec<serde_json::Value>,
    ) -> BoxEndpoint {
        match self.stream_handlers.get(&method) {
            Some(handler) => handler(args),
            None => {
                tracing::warn!(method = ?method.join("."), "missing stream method");
                error_endpoint(Error::method_not_found(&method))
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

fn error_endpoint(error: Error) -> BoxEndpoint {
    let sink = futures::sink::drain().sink_map_err(|infallible| match infallible {});
    let source = futures::stream::once(async move { Err(error) });
    (source.boxed(), Box::pin(sink))
}

fn sink_to_endpoint(
    sink: impl Sink<StreamItem, Error = SinkError> + Send + 'static,
) -> BoxEndpoint {
    let (response_sender, response_receiver) =
        futures::channel::oneshot::channel::<Result<Body, Error>>();
    let source = crate::utils::OneshotStream::new(response_receiver);
    let duplex_sink = sink
        .with::<_, _, _, SinkError>(|item| {
            futures::future::ready({
                match item {
                    StreamItem::Data(_) => Ok(item),
                    StreamItem::Error(err) => Err(SinkError::Error(err)),
                    StreamItem::End => Err(SinkError::Done),
                }
            })
        })
        .sink_map_err(|err| match err {
            SinkError::Done => drop(response_sender),
            SinkError::Error(err) => response_sender.send(Err(err)).unwrap(),
        });
    (source.boxed(), Box::pin(duplex_sink))
}

fn stream_to_endpoint(
    source: impl Stream<Item = Result<Body, Error>> + Send + 'static,
) -> BoxEndpoint {
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
                    Ok(item) => match item {
                        StreamItem::Data(_) => Some(Err(Error {
                            name: "SENT_DATA_TO_SOURCE".to_string(),
                            message: "Cannot send data to a \"source\" stream".to_string(),
                        })),
                        StreamItem::Error(error) => Some(Err(error)),
                        StreamItem::End => None,
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
        Box::pin(crate::utils::OneshotSink::new(peer_end_sender).sink_map_err(|_| ())),
    )
}
