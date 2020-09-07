use anyhow::Context;
use futures::prelude::*;

use super::endpoint::Endpoint;
use super::packet::Body;
use super::server::{AsyncResponse, RpcStreamItem, Server};

struct TestRequestHandler;

impl Server for TestRequestHandler {
    fn handle_async(
        &self,
        method: Vec<String>,
        args: Vec<serde_json::Value>,
    ) -> future::BoxFuture<'static, AsyncResponse> {
        async move {
            let mut args = args;
            match method.as_slice() {
                [m] => match m.as_ref() {
                    "echo" => AsyncResponse::json_ok(&args[0]),
                    "errorAsync" => {
                        let arg = args.pop().unwrap();
                        let echo_error = serde_json::from_value::<EchoError>(arg).unwrap();
                        AsyncResponse::Err {
                            name: echo_error.name,
                            message: echo_error.message,
                        }
                    }
                    m => AsyncResponse::method_not_found(&[m.to_string()]),
                },
                ms => AsyncResponse::method_not_found(ms),
            }
        }
        .boxed()
    }

    fn handle_source(
        &self,
        method: Vec<String>,
        args: Vec<serde_json::Value>,
    ) -> stream::BoxStream<'static, super::server::RpcStreamItem> {
        let mut args = args;
        match method.as_slice() {
            [m] => match m.as_ref() {
                "echoSource" => {
                    let arg = args.pop().unwrap();
                    let values = serde_json::from_value::<Vec<serde_json::Value>>(arg).unwrap();
                    futures::stream::iter(values)
                        .map(|value| RpcStreamItem::Data(Body::json(&value)))
                        .boxed()
                }
                _ => todo!("TestRequestHandler::handle_source"),
            },
            _ => todo!("TestRequestHandler::handle_source"),
        }
    }
}

#[derive(Debug, serde::Deserialize)]
struct EchoError {
    name: String,
    message: String,
}

pub async fn run(bind_addr: impl async_std::net::ToSocketAddrs) -> anyhow::Result<()> {
    let listener = async_std::net::TcpListener::bind(bind_addr).await?;
    listener
        .incoming()
        .map_err(anyhow::Error::from)
        .try_for_each_concurrent(100, handle_incoming)
        .await?;
    Ok(())
}

async fn handle_incoming(stream: async_std::net::TcpStream) -> anyhow::Result<()> {
    tracing::info!(addr = ?stream.peer_addr().unwrap(), "connected to client");
    let (read, write) = stream.split();
    let endpoint = Endpoint::new(
        write.into_sink(),
        crate::utils::read_to_stream(read),
        TestRequestHandler,
    );
    endpoint.join().await.context("Endpoint::join failed")?;
    Ok(())
}
