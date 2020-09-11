use futures::lock::Mutex;
use futures::prelude::*;
use std::pin::Pin;
use std::sync::Arc;

use super::service::{Error, StreamItem};
use crate::rpc::base::packet::Response;

type BoxResponseSink = Pin<Box<dyn Sink<Response, Error = anyhow::Error> + Send + 'static>>;

#[derive(Debug, Clone)]
pub struct Responder {
    response_sink: Arc<Mutex<BoxResponseSink>>,
}

impl Responder {
    pub fn new<ResponseSink>(response_sink: ResponseSink) -> Self
    where
        ResponseSink: Sink<Response> + Send + Unpin + Clone + 'static,
        ResponseSink::Error: std::error::Error + Send + Sync + 'static,
    {
        let response_sink = response_sink.sink_map_err(anyhow::Error::from);
        Self {
            response_sink: Arc::new(Mutex::new(Box::pin(response_sink))),
        }
    }

    pub async fn send(&self, msg: Response) -> anyhow::Result<()> {
        tracing::trace!(?msg, "send response");
        let mut response_sink = self.response_sink.lock().await;
        response_sink.send(msg).await?;
        Ok(())
    }

    pub fn stream(&self, id: u32) -> StreamResponder {
        StreamResponder {
            responder: self.clone(),
            id,
        }
    }
}

#[derive(Debug)]
pub struct StreamResponder {
    responder: Responder,
    id: u32,
}

impl StreamResponder {
    pub async fn send_item(&mut self, item: StreamItem) -> anyhow::Result<()> {
        self.responder.send(item.into_response(self.id)).await
    }

    pub async fn end(&mut self) -> anyhow::Result<()> {
        self.responder
            .send(Response::StreamEnd { number: self.id })
            .await
    }

    pub async fn err(&mut self, error: Error) -> anyhow::Result<()> {
        self.send_item(StreamItem::Error(error)).await
    }
}
