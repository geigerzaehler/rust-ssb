use futures::prelude::*;
use std::pin::Pin;
use std::sync::Arc;

use crate::rpc::base::error::Error;
use crate::rpc::base::packet::{Body, Response};
use crate::rpc::base::stream_message::StreamMessage;

type BoxResponseSink = Pin<Box<dyn Sink<Response, Error = anyhow::Error> + Send>>;

#[derive(Debug, Clone)]
pub struct Responder {
    sink: Arc<futures::lock::Mutex<BoxResponseSink>>,
}

impl Responder {
    pub fn new<ResponseSink>(response_sink: ResponseSink) -> Self
    where
        ResponseSink: Sink<Response> + Send + 'static,
        ResponseSink::Error: std::error::Error + Send + Sync + 'static,
    {
        Self {
            sink: Arc::new(futures::lock::Mutex::new(Box::pin(
                response_sink.sink_map_err(anyhow::Error::from),
            ))),
        }
    }

    pub fn stream(&self, id: u32) -> StreamSink {
        StreamSink {
            responder: self.clone(),
            id,
        }
    }

    pub async fn send_stream_message(
        &self,
        stream_id: u32,
        stream_message: StreamMessage,
    ) -> anyhow::Result<()> {
        self.send(stream_message.into_response(stream_id)).await
    }

    pub async fn send(&self, response: Response) -> anyhow::Result<()> {
        let mut inner = self.sink.lock().await;
        inner.send(response).await?;
        Ok(())
    }
}

#[derive(Debug)]
pub struct StreamSink {
    responder: Responder,
    id: u32,
}

impl StreamSink {
    pub async fn send(&self, data: Body) -> anyhow::Result<()> {
        self.responder
            .send_stream_message(self.id, StreamMessage::Data(data))
            .await
    }

    pub async fn close(self) -> anyhow::Result<()> {
        self.responder
            .send_stream_message(self.id, StreamMessage::End)
            .await
    }

    pub async fn error(self, error: Error) -> anyhow::Result<()> {
        self.responder
            .send_stream_message(self.id, StreamMessage::Error(error))
            .await
    }
}
