use futures::prelude::*;

use super::service::{Error, StreamItem};
use crate::rpc::base::packet::Response;

#[derive(Debug)]
struct ResponseMessage {
    response: Response,
    result_sender: Option<futures::channel::oneshot::Sender<Result<(), anyhow::Error>>>,
}

#[derive(Debug, thiserror::Error)]
#[error("Response worker stopped")]
pub struct StoppedError;

#[derive(Debug)]
pub struct ResponseWorker {
    worker: async_std::task::JoinHandle<()>,
    response_sender: futures::channel::mpsc::UnboundedSender<ResponseMessage>,
}

impl ResponseWorker {
    pub fn start<ResponseSink>(response_sink: ResponseSink) -> Self
    where
        ResponseSink: Sink<Response> + Send + Unpin + Clone + 'static,
        ResponseSink::Error: std::error::Error + Send + Sync + 'static,
    {
        let mut response_sink = response_sink;
        let (response_sender, mut response_receiver) = futures::channel::mpsc::unbounded();
        let worker = async_std::task::spawn(async move {
            while let Some(msg) = response_receiver.next().await {
                let ResponseMessage {
                    response,
                    result_sender,
                } = msg;
                tracing::trace!(?response, "send response");
                let response_result = response_sink.send(response).await;
                if let Some(result_sender) = result_sender {
                    let _ = result_sender.send(response_result.map_err(anyhow::Error::from));
                }
            }
        });
        Self {
            worker,
            response_sender,
        }
    }

    pub async fn join(self) {
        let ResponseWorker {
            worker,
            response_sender,
        } = self;
        drop(response_sender);
        worker.await;
    }

    pub fn responder(&self) -> Responder {
        Responder {
            response_sender: self.response_sender.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Responder {
    response_sender: futures::channel::mpsc::UnboundedSender<ResponseMessage>,
}

impl Responder {
    pub async fn send(&mut self, response: Response) -> Result<(), anyhow::Error> {
        let (result_sender, result_receiver) = futures::channel::oneshot::channel();
        self.response_sender.start_send(ResponseMessage {
            response,
            result_sender: Some(result_sender),
        })?;
        result_receiver
            .await
            .unwrap_or_else(|err| Err(anyhow::Error::from(err)))
    }

    pub fn start_send(&mut self, response: Response) -> Result<(), StoppedError> {
        self.response_sender
            .start_send(ResponseMessage {
                response,
                result_sender: None,
            })
            .map_err(|_| StoppedError)
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
