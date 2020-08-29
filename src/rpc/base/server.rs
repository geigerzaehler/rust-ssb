use futures::future::BoxFuture;
use futures::prelude::*;
use futures::stream::BoxStream;

use super::packet::{Body, Request, Response};

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

    fn into_response(self, number: u32) -> Response {
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

pub trait Server: Sync + Send {
    fn handle_async(
        &self,
        method: Vec<String>,
        args: Vec<serde_json::Value>,
    ) -> BoxFuture<'static, AsyncResponse>;

    fn handle_source(
        &self,
        method: Vec<String>,
        args: Vec<serde_json::Value>,
    ) -> BoxStream<'static, Body>;
}

pub async fn run<ResponseSink>(
    request_handler: &impl Server,
    request_stream: impl Stream<Item = Request> + Unpin,
    response_sink: ResponseSink,
) where
    ResponseSink: Sink<Response> + Send + Unpin + Clone + 'static,
    ResponseSink::Error: std::error::Error + Send + Sync + 'static,
{
    request_stream
        .for_each_concurrent(0, |request| {
            let mut response_sink = response_sink.clone();
            async move {
                match request {
                    Request::Async {
                        number,
                        method,
                        args,
                    } => {
                        let response_fut = request_handler.handle_async(method, args);
                        let response = response_fut.await;
                        response_sink
                            .send(response.into_response(number))
                            .await
                            .unwrap();
                    }
                    Request::Stream { .. } => todo!("server::run Request::Stream"),
                }
            }
        })
        .await;
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

    fn handle_source(
        &self,
        _method: Vec<String>,
        _args: Vec<serde_json::Value>,
    ) -> BoxStream<'static, Body> {
        todo!("NoServer::handle_source")
    }
}
