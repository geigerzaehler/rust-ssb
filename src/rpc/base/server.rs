use futures::future::BoxFuture;
use futures::prelude::*;
use futures::stream::BoxStream;

use super::packet::{Body, Request, Response};
pub use super::stream::RpcStreamItem;
use super::stream::{forward_rpc_stream, RequestType, StreamRequest};

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
#[derive(Debug)]
enum RpcStream {
    Source {
        task: async_std::task::JoinHandle<anyhow::Result<()>>,
    },
    #[allow(dead_code)]
    Sink,
    #[allow(dead_code)]
    Duplex,
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
    ) -> BoxStream<'static, RpcStreamItem>;
}

pub async fn run<ResponseSink>(
    request_handler: &impl Server,
    request_stream: impl Stream<Item = Request> + Unpin,
    response_sink: ResponseSink,
) -> anyhow::Result<()>
where
    ResponseSink: Sink<Response> + Send + Unpin + Clone + 'static,
    ResponseSink::Error: std::error::Error + Send + Sync + 'static,
{
    // TODO errors
    let mut streams = std::collections::HashMap::<u32, RpcStream>::new();
    request_stream
        .map(Result::<_, anyhow::Error>::Ok)
        .try_for_each_concurrent(0, move |request| {
            let mut response_sink = response_sink.clone();
            match request {
                Request::Async {
                    number,
                    method,
                    args,
                } => async move {
                    let response = request_handler.handle_async(method, args).await;
                    response_sink.send(response.into_response(number)).await?;
                    Ok(())
                }
                .boxed(),
                Request::StreamItem { number, body } => {
                    #[allow(clippy::collapsible_if)]
                    if let Some(stream) = streams.get(&number) {
                        match &stream {
                            RpcStream::Source { .. } => {
                                todo!("server::Run cannot send to source stream")
                            }
                            _ => todo!("server::run Request::Stream exists"),
                        }
                    } else {
                        let data = body.into_json().unwrap();
                        let StreamRequest { name, type_, args } =
                            serde_json::from_slice(&data).unwrap();
                        let rpc_stream = match type_ {
                            RequestType::Source => {
                                let source = request_handler.handle_source(name, args);
                                let task = async_std::task::spawn(
                                    forward_rpc_stream(number, source, response_sink)
                                        .map_err(anyhow::Error::from),
                                );
                                RpcStream::Source { task }
                            }
                            RequestType::Sink => todo!("server::run RequestType::Sink"),
                            RequestType::Duplex => todo!("server::run RequestType::Duplex"),
                        };
                        streams.insert(number, rpc_stream);
                    }
                    async { Ok(()) }.boxed()
                }
                Request::StreamEnd { number } => {
                    if let Some(stream) = streams.remove(&number) {
                        match stream {
                            RpcStream::Source { task } => async move {
                                task.cancel().await;
                                Ok(())
                            }
                            .boxed(),
                            _ => todo!("server::run Request::Stream exists"),
                        }
                    } else {
                        todo!("server::run Request::StreamEnd no stream found to end")
                    }
                }
                Request::StreamError { .. } => todo!("server::run Request::StreamError"),
            }
        })
        .await?;
    Ok(())
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
    ) -> BoxStream<'static, RpcStreamItem> {
        todo!("NoServer::handle_source")
    }
}
