use anyhow::Context;
use futures::prelude::*;
use xtra::prelude::*;

use super::responder::Responder;
use super::service::{Error, Service, StreamItem};
use super::stream_worker::{RequestMessage, SinkWorker, SourceWorker};
use crate::rpc::base::packet::{Request, Response};

pub async fn run<ResponseSink>(
    service: Service,
    request_stream: impl Stream<Item = Request> + Unpin + 'static + Send,
    response_sink: ResponseSink,
) -> anyhow::Result<()>
where
    ResponseSink: Sink<Response> + Send + Unpin + Clone + 'static,
    ResponseSink::Error: std::error::Error + Send + Sync + 'static,
{
    let mut request_stream = request_stream;
    let responder = Responder::new(response_sink);
    let mut request_dispatcher = RequestDispatcher {
        service,
        responder,
        streams: std::collections::HashMap::new(),
    };
    while let Some(request) = request_stream.next().await {
        request_dispatcher.handle_request(request)?;
    }
    Ok(())
}

struct RequestDispatcher {
    service: Service,
    responder: Responder,
    streams: std::collections::HashMap<u32, xtra::MessageChannel<RequestMessage>>,
}

impl RequestDispatcher {
    fn handle_request(&mut self, msg: Request) -> anyhow::Result<()> {
        tracing::trace!(?msg, "handle request");
        match msg {
            Request::Async {
                number,
                method,
                args,
            } => {
                let response_fut = self.service.handle_async(method, args);
                let responder = self.responder.clone();
                async_std::task::spawn(async move {
                    let response = response_fut.await;
                    responder
                        .send(response.into_response(number))
                        .await
                        .unwrap();
                });
            }
            Request::StreamData { number, body } => {
                if let Some(stream) = self.streams.get(&number) {
                    stream
                        .do_send(RequestMessage(StreamItem::Data(body)))
                        .unwrap();
                } else {
                    let data = body.into_json().context("Failed to parse stream request")?;
                    let StreamRequest { name, type_, args } =
                        serde_json::from_slice(&data).context("Failed to parse stream request")?;
                    let rpc_stream = match type_ {
                        RequestType::Source => {
                            let source = self.service.handle_source(name, args);
                            let responder = self.responder.clone();
                            SourceWorker::start(responder.stream(number), source).into_channel()
                        }
                        RequestType::Sink => {
                            let sink = self.service.handle_sink(name, args);
                            let responder = self.responder.clone();
                            SinkWorker::start(responder.stream(number), sink).into_channel()
                        }
                        RequestType::Duplex => todo!("server::run RequestType::Duplex"),
                    };
                    self.streams.insert(number, rpc_stream);
                }
            }
            Request::StreamEnd { number } => {
                if let Some(stream) = self.streams.remove(&number) {
                    stream.do_send(RequestMessage(StreamItem::End)).unwrap();
                } else {
                    todo!("server::run Request::StreamEnd no stream found to end")
                }
            }
            Request::StreamError {
                number,
                name,
                message,
            } => {
                if let Some(stream) = self.streams.remove(&number) {
                    stream
                        .do_send(RequestMessage(StreamItem::Error(Error { name, message })))
                        .unwrap();
                } else {
                    todo!("server::run Request::StreamError")
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
struct StreamRequest {
    name: Vec<String>,
    #[serde(rename = "type")]
    type_: RequestType,
    args: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RequestType {
    Source,
    Sink,
    Duplex,
}

impl serde::Serialize for RequestType {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::Source => "source",
            Self::Sink => "sink",
            Self::Duplex => "duplex",
        }
        .serialize(serializer)
    }
}

impl<'de> serde::Deserialize<'de> for RequestType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error;

        let value = String::deserialize(deserializer)?;
        match value.as_ref() {
            "source" => Ok(Self::Source),
            "sink" => Ok(Self::Sink),
            "duplex" => Ok(Self::Duplex),
            value => Err(D::Error::invalid_value(
                serde::de::Unexpected::Str(value),
                &"one of \"source\", \"sink\" or \"duplex\"",
            )),
        }
    }
}
