mod dispatcher;
mod responder;
mod stream_worker;

use futures::future::BoxFuture;
use futures::prelude::*;
use futures::stream::BoxStream;
use std::pin::Pin;

pub use super::packet::Body;
use super::packet::Response;

pub use dispatcher::run;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Error {
    pub name: String,
    pub message: String,
}

pub type BoxSource = BoxStream<'static, Result<Body, Error>>;

#[derive(Debug)]
pub enum AsyncResponse {
    Ok { body: Body },
    Err { name: String, message: String },
}

impl AsyncResponse {
    pub fn method_not_found(method: &[String]) -> Self {
        let name = "METHOD_NOT_FOUND".to_string();
        let message = format!("Method \"{}\" not found", method.join("."));
        Self::Err { name, message }
    }

    pub fn json_ok(value: &impl serde::Serialize) -> Self {
        Self::Ok {
            body: Body::json(value),
        }
    }

    pub(super) fn into_response(self, number: u32) -> Response {
        match self {
            AsyncResponse::Ok { body } => Response::AsyncOk { number, body },
            AsyncResponse::Err { name, message } => Response::AsyncErr {
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
    fn into_response(self, number: u32) -> Response {
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

    fn is_end(&self) -> bool {
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

pub type BoxSink = Pin<Box<dyn Sink<StreamItem, Error = SinkError> + Unpin + Send + Sync>>;

pub trait Server: Sync + Send {
    fn handle_async(
        &self,
        method: Vec<String>,
        args: Vec<serde_json::Value>,
    ) -> BoxFuture<'static, AsyncResponse>;

    fn handle_source(&self, method: Vec<String>, args: Vec<serde_json::Value>) -> BoxSource;

    fn handle_sink(&self, method: Vec<String>, args: Vec<serde_json::Value>) -> BoxSink;
}

#[derive(Debug)]
pub struct NoServer;

impl Server for NoServer {
    fn handle_async(
        &self,
        method: Vec<String>,
        _args: Vec<serde_json::Value>,
    ) -> BoxFuture<'static, AsyncResponse> {
        async move { AsyncResponse::method_not_found(&method) }.boxed()
    }

    fn handle_source(&self, _method: Vec<String>, _args: Vec<serde_json::Value>) -> BoxSource {
        todo!("NoServer::handle_source")
    }

    fn handle_sink(&self, _method: Vec<String>, _args: Vec<serde_json::Value>) -> BoxSink {
        todo!("NoServer::handle_sink")
    }
}
