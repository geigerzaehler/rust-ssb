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

pub type BoxSource = BoxStream<'static, Result<Body, Error>>;

pub type BoxSink = Pin<Box<dyn Sink<StreamItem, Error = SinkError> + Unpin + Send + Sync>>;

pub trait Server: Send {
    fn handle_async(
        &self,
        method: Vec<String>,
        args: Vec<serde_json::Value>,
    ) -> BoxFuture<'static, AsyncResponse>;

    fn handle_source(&self, method: Vec<String>, args: Vec<serde_json::Value>) -> BoxSource;

    fn handle_sink(&self, method: Vec<String>, args: Vec<serde_json::Value>) -> BoxSink;
}

pub fn error_sink(error: Error) -> BoxSink {
    Box::pin(
        futures::sink::drain::<StreamItem>()
            .sink_map_err(|infallible| match infallible {})
            .with(move |_| futures::future::ready(Err(SinkError::Error(error.clone())))),
    )
}

pub fn error_source(error: Error) -> BoxSource {
    futures::stream::once(futures::future::ready(Err(error))).boxed()
}

type Handler<T> = Box<dyn Fn(Vec<serde_json::Value>) -> T + Send + 'static>;

pub struct Service {
    async_handlers: HashMap<Vec<String>, Handler<BoxFuture<'static, AsyncResponse>>>,
    source_handlers: HashMap<Vec<String>, Handler<BoxSource>>,
    sink_handlers: HashMap<Vec<String>, Handler<BoxSink>>,
}

impl Service {
    pub fn new() -> Self {
        Self {
            async_handlers: HashMap::new(),
            source_handlers: HashMap::new(),
            sink_handlers: HashMap::new(),
        }
    }
    pub fn add_async<T: serde::de::DeserializeOwned>(
        &mut self,
        method: impl ToString,
        f: impl Fn(T) -> BoxFuture<'static, AsyncResponse> + Send + 'static,
    ) {
        self.async_handlers.insert(
            vec![method.to_string()],
            Box::new(move |args| {
                let args = serde_json::Value::Array(args);
                match serde_json::from_value::<T>(args) {
                    Ok(args) => f(args),
                    Err(error) => futures::future::ready(AsyncResponse::Err(
                        Error::deserialize_arguments(error),
                    ))
                    .boxed(),
                }
            }),
        );
    }

    pub fn add_source<T: serde::de::DeserializeOwned>(
        &mut self,
        method: impl ToString,
        f: impl Fn(T) -> BoxSource + Send + 'static,
    ) {
        self.source_handlers.insert(
            vec![method.to_string()],
            Box::new(move |args| {
                let args = serde_json::Value::Array(args);
                match serde_json::from_value::<T>(args) {
                    Ok(args) => f(args),
                    Err(error) => error_source(Error::deserialize_arguments(error)),
                }
            }),
        );
    }

    pub fn add_sink<T: serde::de::DeserializeOwned>(
        &mut self,
        method: impl ToString,
        f: impl Fn(T) -> BoxSink + Send + 'static,
    ) {
        self.sink_handlers.insert(
            vec![method.to_string()],
            Box::new(move |args| {
                let args = serde_json::Value::Array(args);
                match serde_json::from_value::<T>(args) {
                    Ok(args) => f(args),
                    Err(error) => error_sink(Error::deserialize_arguments(error)),
                }
            }),
        );
    }

    pub(super) fn handle_async(
        &self,
        method: Vec<String>,
        args: Vec<serde_json::Value>,
    ) -> BoxFuture<'static, AsyncResponse> {
        match self.async_handlers.get(&method) {
            Some(handler) => handler(args),
            None => {
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
            None => error_source(Error::method_not_found(&method)),
        }
    }

    pub(super) fn handle_sink(&self, method: Vec<String>, args: Vec<serde_json::Value>) -> BoxSink {
        match self.sink_handlers.get(&method) {
            Some(handler) => handler(args),
            None => error_sink(Error::method_not_found(&method)),
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
