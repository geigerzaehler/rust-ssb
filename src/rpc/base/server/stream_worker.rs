use futures::prelude::*;
use xtra::prelude::*;

use super::{responder::StreamResponder, BoxDuplex, BoxDuplexSink};
use super::{BoxSink, BoxSource, SinkError, StreamItem};

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

#[derive(Debug)]
pub struct SourceWorker {
    responder: StreamResponder,
}

impl SourceWorker {
    pub(super) fn start(responder: StreamResponder, source: BoxSource) -> xtra::Address<Self> {
        let addr = Self { responder }.spawn();

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

impl xtra::Actor for SourceWorker {}

#[async_trait::async_trait]
impl xtra::Handler<RequestMessage> for SourceWorker {
    async fn handle(&mut self, message: RequestMessage, ctx: &mut Context<Self>) {
        match message.0 {
            StreamItem::Data(_) => todo!("Invalid data"),
            StreamItem::Error(error) => {
                self.responder.err(error).await.unwrap();
                ctx.stop();
            }
            StreamItem::End => {
                self.responder.end().await.unwrap();
                ctx.stop();
            }
        }
    }
}

#[async_trait::async_trait]
impl xtra::Handler<SourceMessage> for SourceWorker {
    async fn handle(&mut self, message: SourceMessage, _ctx: &mut Context<Self>) {
        self.responder.send_item(message.0).await.unwrap();
    }
}

pub struct SinkWorker {
    responder: StreamResponder,
    sink: BoxSink,
}

impl SinkWorker {
    pub(super) fn start(responder: StreamResponder, sink: BoxSink) -> xtra::Address<Self> {
        Self { responder, sink }.spawn()
    }
}

impl xtra::Actor for SinkWorker {}

#[async_trait::async_trait]
impl xtra::Handler<RequestMessage> for SinkWorker {
    async fn handle(&mut self, message: RequestMessage, ctx: &mut Context<Self>) {
        match self.sink.send(message.0).await {
            Ok(()) => {}
            Err(SinkError::Error(error)) => {
                self.responder.err(error).await.unwrap();
                ctx.stop();
            }
            Err(SinkError::Done) => {
                self.responder.end().await.unwrap();
                ctx.stop();
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
            .unwrap_or_else(|e| e.into_any())
    }
}

#[async_trait::async_trait]
impl xtra::Handler<SourceMessage> for DuplexWorker {
    async fn handle(&mut self, message: SourceMessage, _ctx: &mut Context<Self>) {
        self.responder.send_item(message.0).await.unwrap();
    }
}
