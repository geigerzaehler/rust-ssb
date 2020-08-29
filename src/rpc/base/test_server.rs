use futures::prelude::*;

use super::endpoint::Endpoint;
use super::server::{AsyncResponse, Server};

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
        _method: Vec<String>,
        _args: Vec<serde_json::Value>,
    ) -> stream::BoxStream<'static, super::packet::Body> {
        todo!("TestRequestHandler::handle_sync")
    }
}

#[derive(Debug, serde::Deserialize)]
struct EchoError {
    name: String,
    message: String,
}

pub async fn run(bind_addr: impl async_std::net::ToSocketAddrs) -> anyhow::Result<()> {
    let listener = async_std::net::TcpListener::bind(bind_addr).await?;
    loop {
        let (stream, addr) = listener.accept().await?;
        tracing::info!(?addr, "accepted connection");
        async_std::task::spawn(handle_incoming(stream, addr));
    }
}

async fn handle_incoming(stream: async_std::net::TcpStream, addr: std::net::SocketAddr) {
    if let Err(err) = handle_incoming_(stream, addr).await {
        tracing::error!("{:#?}", err)
    }
}

async fn handle_incoming_(
    stream: async_std::net::TcpStream,
    addr: std::net::SocketAddr,
) -> anyhow::Result<()> {
    tracing::info!(?addr, "connected to client");
    let (read, write) = stream.split();
    let endpoint = Endpoint::new(
        write.into_sink(),
        crate::utils::read_to_stream(read),
        TestRequestHandler,
    );
    endpoint.join().await?;
    Ok(())
}
