use futures::prelude::*;
use xtra::prelude::*;

use super::{responder::StreamResponder, BoxDuplex, BoxDuplexSink};
use super::{BoxSink, BoxSource, Error, SinkError, StreamItem};

#[derive(Debug)]
pub struct RequestMessage(pub StreamItem);

impl xtra::Message for RequestMessage {
    type Result = ();
}

#[derive(Debug)]
struct SourceMessage(StreamItem);

impl xtra::Message for SourceMessage {
    type Result = ();
}

pub struct SourceWorker {
    responder: StreamResponder,
    source_reader: Option<async_std::task::JoinHandle<()>>,
    source: Option<BoxSource>,
    source_ended: bool,
}

impl SourceWorker {
    pub(super) fn start(responder: StreamResponder, source: BoxSource) -> xtra::Address<Self> {
        Self {
            responder,
            source: Some(source),
            source_reader: None,
            source_ended: false,
        }
        .spawn()
    }
}

impl xtra::Actor for SourceWorker {
    fn started(&mut self, ctx: &mut Context<Self>) {
        let addr = ctx.address().unwrap();
        let mut source = self.source.take().unwrap();
        self.source_reader = Some(async_std::task::spawn(async move {
            loop {
                let item = source.next().await;
                let item = StreamItem::from(item);
                let is_end = item.is_end();
                let result = addr.do_send(SourceMessage(item));
                if result.is_err() || is_end {
                    break;
                }
            }
        }));
    }

    fn stopped(&mut self, _ctx: &mut Context<Self>) {
        async_std::task::spawn(self.source_reader.take().unwrap().cancel());
    }
}

#[async_trait::async_trait]
impl xtra::Handler<RequestMessage> for SourceWorker {
    async fn handle(&mut self, message: RequestMessage, ctx: &mut Context<Self>) {
        match message.0 {
            StreamItem::Data(_) => {
                if !self.source_ended {
                    self.responder
                        .err(Error {
                            name: "SENT_DATA_TO_SOURCE".to_string(),
                            message: "Cannot send data to a \"source\" stream".to_string(),
                        })
                        .await
                        .unwrap();
                }
                ctx.stop();
            }
            StreamItem::Error(error) => {
                if !self.source_ended {
                    self.responder.err(error).await.unwrap();
                }
                ctx.stop();
            }
            StreamItem::End => {
                if !self.source_ended {
                    self.responder.end().await.unwrap();
                }
                ctx.stop();
            }
        }
    }
}

#[async_trait::async_trait]
impl xtra::Handler<SourceMessage> for SourceWorker {
    async fn handle(&mut self, message: SourceMessage, _ctx: &mut Context<Self>) {
        if message.0.is_end() {
            self.source_ended = true;
        }
        self.responder.send_item(message.0).await.unwrap();
    }
}

pub struct SinkWorker {
    responder: StreamResponder,
    sink: BoxSink,
    sent_end: bool,
}

impl SinkWorker {
    pub(super) fn start(responder: StreamResponder, sink: BoxSink) -> xtra::Address<Self> {
        Self {
            responder,
            sink,
            sent_end: false,
        }
        .spawn()
    }
}

impl xtra::Actor for SinkWorker {}

#[async_trait::async_trait]
impl xtra::Handler<RequestMessage> for SinkWorker {
    async fn handle(&mut self, message: RequestMessage, _ctx: &mut Context<Self>) {
        if (self.sent_end) {
            return;
        }

        let is_end = message.0.is_end();
        let result = self.sink.send(message.0).await;

        match result {
            Ok(()) => {
                if is_end {
                    self.sent_end = true;
                    self.responder.end().await.unwrap();
                }
            }
            Err(SinkError::Error(error)) => {
                self.sent_end = true;
                self.responder.err(error).await.unwrap();
            }
            Err(SinkError::Done) => {
                self.sent_end = true;
                self.responder.end().await.unwrap();
            }
        }
    }
}

pub struct DuplexWorker {
    responder: StreamResponder,
    sink: BoxDuplexSink,
}

impl DuplexWorker {
    pub(super) fn start(responder: StreamResponder, duplex: BoxDuplex) -> xtra::Address<Self> {
        let (source, sink) = duplex;
        let addr = Self { responder, sink }.spawn();

        let addr2 = addr.clone();
        async_std::task::spawn(async move {
            let mut source = source;
            loop {
                let item = source.next().await;
                let item = StreamItem::from(item);
                let is_end = item.is_end();
                let result = addr2.do_send(SourceMessage(item));
                if result.is_err() || is_end {
                    break;
                }
            }
        });

        addr
    }
}

impl xtra::Actor for DuplexWorker {}

#[async_trait::async_trait]
impl xtra::Handler<RequestMessage> for DuplexWorker {
    async fn handle(&mut self, message: RequestMessage, _ctx: &mut Context<Self>) {
        self.sink
            .send(message.0)
            .await
            .unwrap_or_else(|e| e.into_any());
    }
}

#[async_trait::async_trait]
impl xtra::Handler<SourceMessage> for DuplexWorker {
    async fn handle(&mut self, message: SourceMessage, _ctx: &mut Context<Self>) {
        self.responder.send_item(message.0).await.unwrap();
    }
}
