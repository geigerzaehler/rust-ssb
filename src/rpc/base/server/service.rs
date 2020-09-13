use futures::future::BoxFuture;
use futures::prelude::*;
use futures::stream::BoxStream;
use std::collections::HashMap;
use std::pin::Pin;

use crate::rpc::base::packet::{Body, Response};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Error {
    pub name: String,
    pub message: String,
}

impl Error {
    pub fn method_not_found(method: &[String]) -> Self {
        let name = "METHOD_NOT_FOUND".to_string();
        let message = format!("Method \"{}\" not found", method.join("."));
        Self { name, message }
    }

    pub fn deserialize_arguments(error: serde_json::Error) -> Self {
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
    pub fn into_response(self, number: u32) -> Response {
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

    pub fn is_end(&self) -> bool {
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

pub(super) type BoxSource = BoxStream<'static, Result<Body, Error>>;

pub(super) type BoxSink = Pin<Box<dyn Sink<StreamItem, Error = SinkError> + Send>>;

pub(super) type BoxDuplexSink = Pin<Box<dyn Sink<StreamItem, Error = never::Never> + Send>>;

pub(super) type BoxDuplex = (BoxSource, BoxDuplexSink);

pub fn error_sink(error: Error) -> impl Sink<StreamItem, Error = SinkError> {
    Box::pin(
        futures::sink::drain::<StreamItem>()
            .sink_map_err(|infallible| match infallible {})
            .with(move |_| futures::future::ready(Err(SinkError::Error(error.clone())))),
    )
}

pub fn error_source(error: Error) -> impl Stream<Item = Result<Body, Error>> + Send + 'static {
    futures::stream::once(futures::future::ready(Err(error)))
}

type Handler<T> = Box<dyn Fn(Vec<serde_json::Value>) -> T + Send + 'static>;

#[derive(Default)]
pub struct Service {
    async_handlers: HashMap<Vec<String>, Handler<BoxFuture<'static, AsyncResponse>>>,
    source_handlers: HashMap<Vec<String>, Handler<BoxSource>>,
    sink_handlers: HashMap<Vec<String>, Handler<BoxSink>>,
    duplex_handlers: HashMap<Vec<String>, Handler<BoxDuplex>>,
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
        self.source_handlers.insert(
            vec![method.to_string()],
            Box::new(move |args| {
                let args = serde_json::Value::Array(args);
                match serde_json::from_value::<Args>(args) {
                    Ok(args) => f(args).boxed(),
                    Err(error) => error_source(Error::deserialize_arguments(error)).boxed(),
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
        self.sink_handlers.insert(
            vec![method.to_string()],
            Box::new(move |args| {
                let args = serde_json::Value::Array(args);
                match serde_json::from_value::<Args>(args) {
                    Ok(args) => Box::pin(f(args)),
                    Err(error) => Box::pin(error_sink(Error::deserialize_arguments(error))),
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
        Sink_: Sink<StreamItem, Error = never::Never> + Send + 'static,
    {
        let method2 = method.to_string();
        self.duplex_handlers.insert(
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
                        let source = error_source(Error::deserialize_arguments(error))
                        .boxed();
                        let sink = Box::pin(futures::sink::drain::<StreamItem>().sink_map_err(|infallible| match infallible {}));
                        (source, sink)
                    }
                }
            }),
        );
    }

    pub fn add_service(&mut self, group: impl ToString, service: Self) {
        let Self {
            async_handlers,
            source_handlers,
            sink_handlers,
            duplex_handlers,
        } = service;
        self.async_handlers
            .extend(async_handlers.into_iter().map(|(mut k, v)| {
                k.insert(0, group.to_string());
                (k, v)
            }));
        self.source_handlers
            .extend(source_handlers.into_iter().map(|(mut k, v)| {
                k.insert(0, group.to_string());
                (k, v)
            }));
        self.sink_handlers
            .extend(sink_handlers.into_iter().map(|(mut k, v)| {
                k.insert(0, group.to_string());
                (k, v)
            }));
        self.duplex_handlers
            .extend(duplex_handlers.into_iter().map(|(mut k, v)| {
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

    pub(super) fn handle_source(
        &self,
        method: Vec<String>,
        args: Vec<serde_json::Value>,
    ) -> BoxSource {
        match self.source_handlers.get(&method) {
            Some(handler) => handler(args),
            None => {
                tracing::warn!(method = ?method.join("."), "missing source");
                Box::pin(error_source(Error::method_not_found(&method)))
            }
        }
    }

    pub(super) fn handle_sink(&self, method: Vec<String>, args: Vec<serde_json::Value>) -> BoxSink {
        match self.sink_handlers.get(&method) {
            Some(handler) => handler(args),
            None => {
                tracing::warn!(method = ?method.join("."), "missing sink");
                Box::pin(error_sink(Error::method_not_found(&method)))
            }
        }
    }

    pub(super) fn handle_duplex(
        &self,
        method: Vec<String>,
        args: Vec<serde_json::Value>,
    ) -> BoxDuplex {
        match self.duplex_handlers.get(&method) {
            Some(handler) => handler(args),
            None => {
                tracing::warn!(method = ?method.join("."), "missing duplex");
                let source = error_source(Error::method_not_found(&method)).boxed();
                let sink =
                    Box::pin(futures::sink::drain().sink_map_err(|infallible| match infallible {}));
                (source, sink)
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
                "source_handlers",
                &self.source_handlers.keys().collect::<Vec<_>>(),
            )
            .field(
                "sink_handlers",
                &self.sink_handlers.keys().collect::<Vec<_>>(),
            )
            .finish()
    }
}
