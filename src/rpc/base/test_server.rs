use anyhow::Context;
use futures::prelude::*;

use super::endpoint::Endpoint;
use super::server::{AsyncResponse, Body, Service, SinkError};
use super::{Error, StreamItem};

fn test_service() -> Service {
    let mut service = Service::new();

    service.add_async("asyncEcho", |(x,): (serde_json::Value,)| async move {
        AsyncResponse::json_ok(&x)
    });

    service.add_async("asyncError", |(error,): (EchoError,)| async move {
        AsyncResponse::Err(Error {
            name: error.name,
            message: error.message,
        })
    });

    service.add_source("sourceEcho", |(values,): (Vec<serde_json::Value>,)| {
        futures::stream::iter(values).map(|value| Ok(Body::json(&value)))
    });

    service.add_source(
        "sourceError",
        |(_, error): (serde_json::Value, EchoError)| {
            futures::stream::once(async move {
                Err(Error {
                    name: error.name,
                    message: error.message,
                })
            })
        },
    );

    service.add_source("sourceInifite", |_: Vec<()>| {
        futures::stream::unfold((), |()| async {
            async_std::task::sleep(std::time::Duration::from_millis(1)).await;
            Some((Ok(Body::json(&0)), ()))
        })
    });

    service.add_sink("sinkExpect", |(values,): (Vec<serde_json::Value>,)| {
        let mut collected = Vec::<serde_json::Value>::new();
        futures::sink::drain()
            .sink_map_err(|infallible| match infallible {})
            .with(move |item: StreamItem| {
                futures::future::ready(match item {
                    StreamItem::Data(body) => {
                        let item = body.decode_json::<serde_json::Value>().unwrap();
                        collected.push(item);
                        Ok(())
                    }
                    StreamItem::Error { .. } => Err(SinkError::Done),
                    StreamItem::End => {
                        if collected == values {
                            Err(SinkError::Done)
                        } else {
                            Err(SinkError::Error(Error {
                                name: "Unexpected error".to_string(),
                                message: "".to_string(),
                            }))
                        }
                    }
                })
            })
    });

    service.add_sink("sinkAbortError", |(n, error): (u32, EchoError)| {
        let mut remaining_items = n;
        futures::sink::drain()
            .sink_map_err(|infallible| match infallible {})
            .with(move |item: StreamItem| {
                futures::future::ready(match item {
                    StreamItem::Data(_) => {
                        remaining_items -= 1;
                        if remaining_items == 0 {
                            Err(SinkError::Error(Error {
                                name: error.name.clone(),
                                message: error.message.clone(),
                            }))
                        } else {
                            Ok(())
                        }
                    }
                    StreamItem::Error { .. } => Err(SinkError::Done),
                    _ => Err(SinkError::Error(Error {
                        name: "Unexpected end or error".to_string(),
                        message: "".to_string(),
                    })),
                })
            })
    });

    service.add_duplex("duplexAdd", |(summand,): (u64,)| {
        let (incoming_sink, incoming) = futures::channel::mpsc::unbounded();
        // This should never panic. `incoming` is only dropped after we stop accepting inputs on `sink`.
        let sink = incoming_sink.sink_map_err(|err| panic!("{}", err));

        let source = incoming.scan(false, move |closed, item| {
            if *closed {
                return futures::future::ready(None);
            }
            let result = match item {
                StreamItem::Data(body) => {
                    let value = body.decode_json::<u64>().unwrap();
                    Some(Ok(Body::json(&(value + summand))))
                }
                StreamItem::Error(err) => {
                    *closed = true;
                    Some(Err(err))
                }
                StreamItem::End => {
                    *closed = true;
                    None
                }
            };
            futures::future::ready(result)
        });
        (source, sink)
    });

    service
}

#[derive(Debug, Clone, serde::Deserialize)]
struct EchoError {
    name: String,
    message: String,
}

pub async fn run(bind_addr: impl async_std::net::ToSocketAddrs) -> anyhow::Result<()> {
    let listener = async_std::net::TcpListener::bind(bind_addr).await?;
    listener
        .incoming()
        .map_err(anyhow::Error::from)
        .try_for_each_concurrent(100, |addr| async move {
            std::panic::AssertUnwindSafe(handle_incoming(addr))
                .catch_unwind()
                .await
                .unwrap_or_else(|_| Err(anyhow::anyhow!("client handler panicked")))
        })
        .await?;
    Ok(())
}

async fn handle_incoming(stream: async_std::net::TcpStream) -> anyhow::Result<()> {
    tracing::info!(addr = ?stream.peer_addr().unwrap(), "connected to client");
    let (read, write) = stream.split();
    let endpoint = Endpoint::new(
        write.into_sink(),
        crate::utils::read_to_stream(read),
        test_service(),
    );
    endpoint.join().await.context("Endpoint::join failed")?;
    Ok(())
}
