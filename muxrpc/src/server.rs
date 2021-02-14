use anyhow::Context as _;
use futures::prelude::*;

use super::packet::{Request, Response};
use super::service::{BoxEndpointSink, BoxEndpointStream, Error, Service, StreamMessage};
use super::stream_request::StreamRequest;

pub async fn run(
    service: Service,
    request_stream: impl Stream<Item = Request> + Unpin + 'static + Send,
    response_sender: futures::channel::mpsc::Sender<Response>,
) -> anyhow::Result<()> {
    let mut request_stream = request_stream;
    let mut request_dispatcher = RequestDispatcher {
        service,
        response_sender,
        streams: std::collections::HashMap::new(),
    };
    while let Some(request) = request_stream.next().await {
        request_dispatcher.handle_request(request)?;
    }
    Ok(())
}

struct RequestDispatcher {
    service: Service,
    response_sender: futures::channel::mpsc::Sender<Response>,
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
                let mut response_sender = self.response_sender.clone();
                async_std::task::spawn(async move {
                    let response = response_fut.await;
                    let result = response_sender.send(response.into_response(number)).await;
                    if let Err(error) = result {
                        tracing::warn!(response_id = ?number, ?error, "Failed to send response");
                    }
                });
            }
            Request::Stream { number, message } => match message {
                StreamMessage::Data(body) => {
                    if let Some(stream) = self.streams.get_mut(&number) {
                        stream.incoming(StreamMessage::Data(body));
                    } else {
                        let StreamRequest { name, type_, args } = body
                            .decode_json()
                            .context("Failed to parse stream request")?;
                        tracing::debug!(name = ?name.join("."), ?type_, "stream request");
                        let (source, sink) = self.service.handle_stream(name, args);
                        let stream_handle =
                            StreamHandle::new(number, self.response_sender.clone(), source, sink);
                        self.streams.insert(number, stream_handle);
                    }
                }
                StreamMessage::Error(_) | StreamMessage::End => {
                    if let Some(mut stream) = self.streams.remove(&number) {
                        stream.incoming(message);
                    } else {
                        let mut response_sender = self.response_sender.clone();
                        async_std::task::spawn(async move {
                            // We don’t care if the connection has been dropped
                            let _ = response_sender
                                .send(
                                    StreamMessage::Error(Error {
                                        name: "STREAM_DOES_NOT_EXIST".to_string(),
                                        message: format!(
                                            "Stream with ID {:?} does not exist",
                                            number
                                        ),
                                    })
                                    .into_response(number),
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

/// Handle for the dipsatcher to communicate with the stream created by [Service].
struct StreamHandle {
    incoming_sender: futures::channel::mpsc::UnboundedSender<StreamMessage>,
}

impl StreamHandle {
    fn new(
        stream_id: u32,
        response_sink: futures::channel::mpsc::Sender<Response>,
        source: BoxEndpointStream,
        sink: BoxEndpointSink,
    ) -> Self {
        let (incoming_sender, incoming_receiver) =
            futures::channel::mpsc::unbounded::<StreamMessage>();

        async_std::task::spawn(async move {
            let mut source = source;
            let mut response_sink = response_sink;
            loop {
                let item = source.next().await;
                let message = match item {
                    None => StreamMessage::End,
                    Some(Ok(body)) => StreamMessage::Data(body),
                    Some(Err(error)) => StreamMessage::Error(error),
                };
                let message_is_end = message.is_end();
                let result = response_sink.send(message.into_response(stream_id)).await;
                if result.is_err() || message_is_end {
                    break;
                }
            }
        });

        async_std::task::spawn(async move {
            let _ = incoming_receiver.map(Ok).forward(sink).await;
        });

        Self { incoming_sender }
    }

    fn incoming(&mut self, stream_message: StreamMessage) {
        let _ = self.incoming_sender.unbounded_send(stream_message);
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::packet::Body;
    use crate::stream_request::{StreamRequest, StreamRequestType};

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
                    type_: StreamRequestType::Source,
                    args: vec![],
                }
                .into_request(1),
            )
            .await;

        let response = test_dispatcher.recv().await.unwrap();
        assert_eq!(response, StreamMessage::End.into_response(1));
        test_dispatcher
            .send(StreamMessage::End.into_request(1))
            .await;

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
                    type_: StreamRequestType::Source,
                    args: vec![],
                }
                .into_request(1),
            )
            .await;
        test_dispatcher
            .send(StreamMessage::End.into_request(1))
            .await;
        let responses = test_dispatcher.end().await;
        assert_eq!(responses, vec![StreamMessage::End.into_response(1)]);
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
                    type_: StreamRequestType::Source,
                    args: vec![],
                }
                .into_request(1),
            )
            .await;
        test_dispatcher
            .send(StreamMessage::End.into_request(1))
            .await;
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
                    type_: StreamRequestType::Source,
                    args: vec![],
                }
                .into_request(1),
            )
            .await;
        test_dispatcher
            .send(StreamMessage::End.into_request(1))
            .await;
        let responses = test_dispatcher.end().await;
        assert_eq!(responses, vec![StreamMessage::End.into_response(1)]);
    }

    #[async_std::test]
    async fn sink_end_server() {
        let _ = tracing_subscriber::fmt::try_init();

        let mut service = Service::new();
        service.add_sink("sink", |_: Vec<()>| {
            futures::sink::drain::<StreamMessage>()
                .sink_map_err(|infallible| match infallible {})
                .with(|_| futures::future::ready(Err(super::super::service::SinkError::Done)))
        });

        let mut test_dispatcher = TestDispatcher::new(service);

        test_dispatcher
            .send(
                StreamRequest {
                    name: vec!["sink".to_string()],
                    type_: StreamRequestType::Source,
                    args: vec![],
                }
                .into_request(1),
            )
            .await;
        test_dispatcher
            .send(StreamMessage::Data(Body::String("".to_string())).into_request(1))
            .await;
        test_dispatcher
            .send(StreamMessage::Data(Body::String("".to_string())).into_request(1))
            .await;
        let response = test_dispatcher.recv().await.unwrap();
        assert_eq!(response, StreamMessage::End.into_response(1));
        test_dispatcher
            .send(StreamMessage::End.into_request(1))
            .await;

        let responses = test_dispatcher.end().await;
        assert_eq!(responses, vec![]);
    }

    #[async_std::test]
    async fn end_msg_after_end() {
        let _ = tracing_subscriber::fmt::try_init();

        let mut service = Service::new();
        service.add_source("source", |_: Vec<()>| futures::stream::pending());

        let mut test_dispatcher = TestDispatcher::new(service);

        test_dispatcher
            .send(StreamMessage::End.into_request(1))
            .await;
        test_dispatcher
            .send(
                StreamMessage::Error(Error {
                    name: "".to_string(),
                    message: "".to_string(),
                })
                .into_request(2),
            )
            .await;
        let responses = test_dispatcher.end().await;
        assert_eq!(responses.len(), 2);
        assert!(responses.contains(
            &StreamMessage::Error(Error {
                name: "STREAM_DOES_NOT_EXIST".to_string(),
                message: "Stream with ID 1 does not exist".to_string()
            })
            .into_response(1)
        ));
        assert!(responses.contains(
            &StreamMessage::Error(Error {
                name: "STREAM_DOES_NOT_EXIST".to_string(),
                message: "Stream with ID 2 does not exist".to_string()
            })
            .into_response(2)
        ));
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
                    type_: StreamRequestType::Source,
                    args: vec![],
                }
                .into_request(1),
            )
            .await;
        test_dispatcher
            .send(StreamMessage::Data(Body::String("".to_string())).into_request(1))
            .await;
        let responses = test_dispatcher.end().await;
        assert_eq!(
            responses,
            vec![StreamMessage::Error(Error {
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
                    type_: StreamRequestType::Source,
                    args: vec![],
                }
                .into_request(1),
            )
            .await;
        test_dispatcher
            .send(
                StreamRequest {
                    name: vec!["sink".to_string()],
                    type_: StreamRequestType::Source,
                    args: vec![],
                }
                .into_request(1),
            )
            .await;
        test_dispatcher.close_connection();
        test_dispatcher.end().await;
    }

    struct TestDispatcher {
        request_sender: futures::channel::mpsc::Sender<Request>,
        response_receiver: futures::channel::mpsc::Receiver<Response>,
        run_handle: async_std::task::JoinHandle<Result<(), anyhow::Error>>,
    }

    impl TestDispatcher {
        fn new(service: Service) -> Self {
            let (request_sender, request_receiver) = futures::channel::mpsc::channel(10);
            let (response_sender, response_receiver) = futures::channel::mpsc::channel(10);

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
