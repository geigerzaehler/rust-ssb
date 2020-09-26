use anyhow::Context;
use futures::prelude::*;

use super::responder::{Responder, StreamSink};
use super::service::{BoxEndpoint, Service};
use crate::rpc::base::packet::{Request, Response};
use crate::rpc::base::stream_request::StreamRequest;
use crate::rpc::base::{Error, StreamItem};

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
    streams: std::collections::HashMap<u32, StreamHandle>,
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
            Request::StreamItem { number, item } => match item {
                StreamItem::Data(body) => {
                    if let Some(stream) = self.streams.get_mut(&number) {
                        stream.send(StreamItem::Data(body));
                    } else {
                        let data = body.into_json().context("Failed to parse stream request")?;
                        let StreamRequest { name, type_, args } = serde_json::from_slice(&data)
                            .context("Failed to parse stream request")?;
                        let responder = self.responder.clone();
                        tracing::debug!(name = ?name.join("."), ?type_, "stream request");
                        let source = self.service.handle_stream(name, args);
                        let stream_handle = StreamHandle::new(responder.stream(number), source);
                        self.streams.insert(number, stream_handle);
                    }
                }
                StreamItem::Error(_) | StreamItem::End => {
                    if let Some(mut stream) = self.streams.remove(&number) {
                        stream.send(item);
                    } else {
                        let responder = self.responder.clone();
                        async_std::task::spawn(async move {
                            let _ = responder
                                .send_stream_item(
                                    number,
                                    StreamItem::Error(Error {
                                        name: "STREAM_DOES_NOT_EXIST".to_string(),
                                        message: format!(
                                            "Stream with ID {:?} does not exist",
                                            number
                                        ),
                                    }),
                                )
                                .await;
                        });
                    }
                }
            },
        }
        Ok(())
    }
}

struct StreamHandle {
    input_sender: futures::channel::mpsc::UnboundedSender<StreamItem>,
}

impl StreamHandle {
    fn new(responder: StreamSink, (source, sink): BoxEndpoint) -> Self {
        let (input_sender, input_receiver) = futures::channel::mpsc::unbounded::<StreamItem>();

        async_std::task::spawn(async move {
            let mut source = source;
            loop {
                let item = source.next().await;
                let result = match item {
                    None => {
                        let _ = responder.close().await;
                        break;
                    }
                    Some(Ok(body)) => responder.send(body).await,
                    Some(Err(error)) => {
                        let _ = responder.error(error).await;
                        break;
                    }
                };
                if result.is_err() {
                    break;
                }
            }
        });

        async_std::task::spawn(async move {
            let _ = input_receiver.map(Ok).forward(sink).await;
        });

        Self { input_sender }
    }

    fn send(&mut self, item: StreamItem) {
        let _ = self.input_sender.unbounded_send(item);
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::rpc::base::packet::Body;
    use crate::rpc::base::stream_request::{RequestType, StreamRequest};

    #[async_std::test]
    async fn source_end_server() {
        let _ = tracing_subscriber::fmt::try_init();

        let mut service = Service::new();
        service.add_source("source", |_: Vec<()>| futures::stream::empty());

        let mut test_dispatcher = TestDispatcher::new(service);

        test_dispatcher
            .send(
                StreamRequest {
                    name: vec!["source".to_string()],
                    type_: RequestType::Source,
                    args: vec![],
                }
                .into_request(1),
            )
            .await;

        let response = test_dispatcher.recv().await.unwrap();
        assert_eq!(response, StreamItem::End.into_response(1));
        test_dispatcher.send(StreamItem::End.into_request(1)).await;

        let responses = test_dispatcher.end().await;
        assert_eq!(responses, vec![]);
    }

    #[async_std::test]
    async fn source_end_client() {
        let _ = tracing_subscriber::fmt::try_init();

        let mut service = Service::new();
        service.add_source("source", |_: Vec<()>| futures::stream::pending());

        let mut test_dispatcher = TestDispatcher::new(service);

        test_dispatcher
            .send(
                StreamRequest {
                    name: vec!["source".to_string()],
                    type_: RequestType::Source,
                    args: vec![],
                }
                .into_request(1),
            )
            .await;
        test_dispatcher.send(StreamItem::End.into_request(1)).await;
        let responses = test_dispatcher.end().await;
        assert_eq!(responses, vec![StreamItem::End.into_response(1)]);
    }

    #[async_std::test]
    async fn source_drop_on_end() {
        let _ = tracing_subscriber::fmt::try_init();

        let mut service = Service::new();
        let (source_sender, source) = futures::channel::mpsc::unbounded();
        let source_cell = std::cell::RefCell::new(Some(source));
        service.add_source("source", move |_: Vec<()>| {
            source_cell.borrow_mut().take().unwrap()
        });

        let mut test_dispatcher = TestDispatcher::new(service);

        test_dispatcher
            .send(
                StreamRequest {
                    name: vec!["source".to_string()],
                    type_: RequestType::Source,
                    args: vec![],
                }
                .into_request(1),
            )
            .await;
        test_dispatcher.send(StreamItem::End.into_request(1)).await;
        test_dispatcher.end().await;
        assert!(source_sender.is_closed());
    }

    #[async_std::test]
    async fn sink_end_client() {
        let _ = tracing_subscriber::fmt::try_init();

        let mut service = Service::new();
        service.add_sink("sink", |_: Vec<()>| {
            futures::sink::drain().sink_map_err(|infallible| match infallible {})
        });

        let mut test_dispatcher = TestDispatcher::new(service);

        test_dispatcher
            .send(
                StreamRequest {
                    name: vec!["sink".to_string()],
                    type_: RequestType::Source,
                    args: vec![],
                }
                .into_request(1),
            )
            .await;
        test_dispatcher.send(StreamItem::End.into_request(1)).await;
        let responses = test_dispatcher.end().await;
        assert_eq!(responses, vec![StreamItem::End.into_response(1)]);
    }

    #[async_std::test]
    async fn sink_end_server() {
        let _ = tracing_subscriber::fmt::try_init();

        let mut service = Service::new();
        service.add_sink("sink", |_: Vec<()>| {
            futures::sink::drain::<StreamItem>()
                .sink_map_err(|infallible| match infallible {})
                .with(|_| futures::future::ready(Err(super::super::service::SinkError::Done)))
        });

        let mut test_dispatcher = TestDispatcher::new(service);

        test_dispatcher
            .send(
                StreamRequest {
                    name: vec!["sink".to_string()],
                    type_: RequestType::Source,
                    args: vec![],
                }
                .into_request(1),
            )
            .await;
        test_dispatcher
            .send(StreamItem::Data(Body::String("".to_string())).into_request(1))
            .await;
        test_dispatcher
            .send(StreamItem::Data(Body::String("".to_string())).into_request(1))
            .await;
        let response = test_dispatcher.recv().await.unwrap();
        assert_eq!(response, StreamItem::End.into_response(1));
        test_dispatcher.send(StreamItem::End.into_request(1)).await;

        let responses = test_dispatcher.end().await;
        assert_eq!(responses, vec![]);
    }

    #[async_std::test]
    async fn end_msg_after_end() {
        let _ = tracing_subscriber::fmt::try_init();

        let mut service = Service::new();
        service.add_source("source", |_: Vec<()>| futures::stream::pending());

        let mut test_dispatcher = TestDispatcher::new(service);

        test_dispatcher.send(StreamItem::End.into_request(1)).await;
        test_dispatcher
            .send(
                StreamItem::Error(Error {
                    name: "".to_string(),
                    message: "".to_string(),
                })
                .into_request(2),
            )
            .await;
        let responses = test_dispatcher.end().await;
        assert_eq!(
            responses,
            vec![
                StreamItem::Error(Error {
                    name: "STREAM_DOES_NOT_EXIST".to_string(),
                    message: "Stream with ID 1 does not exist".to_string()
                })
                .into_response(1),
                StreamItem::Error(Error {
                    name: "STREAM_DOES_NOT_EXIST".to_string(),
                    message: "Stream with ID 2 does not exist".to_string()
                })
                .into_response(2)
            ]
        );
    }

    #[async_std::test]
    async fn source_data_request() {
        let _ = tracing_subscriber::fmt::try_init();

        let mut service = Service::new();
        service.add_source("source", |_: Vec<()>| futures::stream::pending());

        let mut test_dispatcher = TestDispatcher::new(service);

        test_dispatcher
            .send(
                StreamRequest {
                    name: vec!["source".to_string()],
                    type_: RequestType::Source,
                    args: vec![],
                }
                .into_request(1),
            )
            .await;
        test_dispatcher
            .send(StreamItem::Data(Body::String("".to_string())).into_request(1))
            .await;
        let responses = test_dispatcher.end().await;
        assert_eq!(
            responses,
            vec![StreamItem::Error(Error {
                name: "SENT_DATA_TO_SOURCE".to_string(),
                message: "Cannot send data to a \"source\" stream".to_string()
            })
            .into_response(1)]
        );
    }

    #[async_std::test]
    async fn connection_closed() {
        let _ = tracing_subscriber::fmt::try_init();

        let mut service = Service::new();
        service.add_source("source", |_: Vec<()>| futures::stream::pending());
        service.add_sink("sink", |_: Vec<()>| {
            futures::sink::drain().sink_map_err(|infallible| match infallible {})
        });

        let mut test_dispatcher = TestDispatcher::new(service);

        test_dispatcher
            .send(
                StreamRequest {
                    name: vec!["source".to_string()],
                    type_: RequestType::Source,
                    args: vec![],
                }
                .into_request(1),
            )
            .await;
        test_dispatcher
            .send(
                StreamRequest {
                    name: vec!["sink".to_string()],
                    type_: RequestType::Source,
                    args: vec![],
                }
                .into_request(1),
            )
            .await;
        test_dispatcher.close_connection();
        test_dispatcher.end().await;
    }

    struct TestDispatcher {
        request_sender: futures::channel::mpsc::UnboundedSender<Request>,
        response_receiver: futures::channel::mpsc::UnboundedReceiver<Response>,
        run_handle: async_std::task::JoinHandle<Result<(), anyhow::Error>>,
    }

    impl TestDispatcher {
        fn new(service: Service) -> Self {
            let (request_sender, request_receiver) = futures::channel::mpsc::unbounded();
            let (response_sender, response_receiver) = futures::channel::mpsc::unbounded();

            let run_handle =
                async_std::task::spawn(run(service, request_receiver, response_sender));

            Self {
                request_sender,
                response_receiver,
                run_handle,
            }
        }

        async fn send(&mut self, request: Request) {
            self.request_sender.send(request).await.unwrap();
        }

        async fn recv(&mut self) -> Option<Response> {
            self.response_receiver.next().await
        }

        fn close_connection(&mut self) {
            self.request_sender.close_channel();
            self.response_receiver.close();
        }

        async fn end(self) -> Vec<Response> {
            let TestDispatcher {
                request_sender,
                response_receiver,
                run_handle,
            } = self;
            drop(request_sender);
            run_handle.await.unwrap();
            response_receiver.collect::<Vec<_>>().await
        }
    }
}
