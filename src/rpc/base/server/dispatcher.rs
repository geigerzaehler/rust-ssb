use anyhow::Context;
use futures::prelude::*;
use xtra::prelude::*;

use super::responder::{Responder, ResponseWorker};
use super::service::{Error, Service, StreamItem};
use super::stream_worker::{DuplexWorker, RequestMessage, SinkWorker, SourceWorker};
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
    let response_worker = ResponseWorker::start(response_sink);
    let mut request_dispatcher = RequestDispatcher {
        service,
        responder: response_worker.responder(),
        streams: std::collections::HashMap::new(),
    };
    while let Some(request) = request_stream.next().await {
        request_dispatcher.handle_request(request)?;
    }
    // TODO stop response worker
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
                let mut responder = self.responder.clone();
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
                    let responder = self.responder.clone();
                    tracing::debug!(name = ?name.join("."), ?type_, "stream request");
                    let rpc_stream = match type_ {
                        RequestType::Source => {
                            let source = self.service.handle_source(name, args);
                            SourceWorker::start(responder.stream(number), source).into_channel()
                        }
                        RequestType::Sink => {
                            let sink = self.service.handle_sink(name, args);
                            SinkWorker::start(responder.stream(number), sink).into_channel()
                        }
                        RequestType::Duplex => {
                            let duplex = self.service.handle_duplex(name, args);
                            DuplexWorker::start(responder.stream(number), duplex).into_channel()
                        }
                    };
                    self.streams.insert(number, rpc_stream);
                }
            }
            Request::StreamEnd { number } => {
                if let Some(stream) = self.streams.remove(&number) {
                    stream.do_send(RequestMessage(StreamItem::End)).unwrap();
                } else {
                    self.responder
                        .start_send(Response::StreamError {
                            number,
                            name: "STREAM_DOES_NOT_EXIST".to_string(),
                            message: format!("Stream with ID {:?} does not exist", number),
                        })
                        .unwrap();
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
                    self.responder
                        .start_send(Response::StreamError {
                            number,
                            name: "STREAM_DOES_NOT_EXIST".to_string(),
                            message: format!("Stream with ID {:?} does not exist", number),
                        })
                        .unwrap();
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

#[cfg(test)]
mod test {
    use super::*;
    use crate::rpc::base::packet::Body;

    #[async_std::test]
    async fn source_end_server() {
        let _ = tracing_subscriber::fmt::try_init();

        let mut service = Service::new();
        service.add_source("source", |_: Vec<()>| futures::stream::empty());

        let (request_sender, request_receiver) = futures::channel::mpsc::unbounded();
        let (response_sender, mut response_receiver) = futures::channel::mpsc::unbounded();

        let run_handle = async_std::task::spawn(run(service, request_receiver, response_sender));

        request_sender
            .unbounded_send(Request::StreamData {
                number: 1,
                body: Body::json(&StreamRequest {
                    name: vec!["source".to_string()],
                    type_: RequestType::Source,
                    args: vec![],
                }),
            })
            .unwrap();

        let response = response_receiver.next().await.unwrap();
        assert_eq!(response, Response::StreamEnd { number: 1 });
        request_sender
            .unbounded_send(Request::StreamEnd { number: 1 })
            .unwrap();
        drop(request_sender);
        assert!(response_receiver.next().await.is_none());

        run_handle.await.unwrap();
    }

    #[async_std::test]
    async fn source_end_client() {
        let _ = tracing_subscriber::fmt::try_init();

        let mut service = Service::new();
        service.add_source("source", |_: Vec<()>| futures::stream::pending());

        let (request_sender, request_receiver) = futures::channel::mpsc::unbounded();
        let (response_sender, response_receiver) = futures::channel::mpsc::unbounded();

        request_sender
            .unbounded_send(Request::StreamData {
                number: 1,
                body: Body::json(&StreamRequest {
                    name: vec!["source".to_string()],
                    type_: RequestType::Source,
                    args: vec![],
                }),
            })
            .unwrap();
        request_sender
            .unbounded_send(Request::StreamEnd { number: 1 })
            .unwrap();
        drop(request_sender);
        run(service, request_receiver, response_sender)
            .await
            .unwrap();
        let responses = response_receiver.collect::<Vec<_>>().await;
        assert_eq!(responses, vec![Response::StreamEnd { number: 1 }]);
    }

    #[async_std::test]
    async fn end_msg_after_end() {
        let _ = tracing_subscriber::fmt::try_init();

        let mut service = Service::new();
        service.add_source("source", |_: Vec<()>| futures::stream::pending());

        let (request_sender, request_receiver) = futures::channel::mpsc::unbounded();
        let (response_sender, response_receiver) = futures::channel::mpsc::unbounded();

        request_sender
            .unbounded_send(Request::StreamEnd { number: 1 })
            .unwrap();
        request_sender
            .unbounded_send(Request::StreamError {
                number: 2,
                name: "".to_string(),
                message: "".to_string(),
            })
            .unwrap();
        drop(request_sender);
        run(service, request_receiver, response_sender)
            .await
            .unwrap();
        let responses = response_receiver.collect::<Vec<_>>().await;
        assert_eq!(
            responses,
            vec![
                Response::StreamError {
                    number: 1,
                    name: "STREAM_DOES_NOT_EXIST".to_string(),
                    message: "Stream with ID 1 does not exist".to_string()
                },
                Response::StreamError {
                    number: 2,
                    name: "STREAM_DOES_NOT_EXIST".to_string(),
                    message: "Stream with ID 2 does not exist".to_string()
                }
            ]
        );
    }
}
