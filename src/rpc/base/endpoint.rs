use futures::prelude::*;

use super::client::Client;
use super::packet::{Packet, Request, Response};
use super::packet_stream::PacketStream;
use super::server::Server;

#[derive(Debug)]
pub struct Endpoint {
    client: Client,
    server_task: async_std::task::JoinHandle<anyhow::Result<()>>,
    packet_reader_task: async_std::task::JoinHandle<anyhow::Result<()>>,
    packet_sender_task: async_std::task::JoinHandle<anyhow::Result<()>>,
}

impl Endpoint {
    pub fn new<Sink_, TryStream_>(
        send: Sink_,
        receive: TryStream_,
        request_handler: impl Server + 'static,
    ) -> Self
    where
        Sink_: Sink<Vec<u8>> + Send + Unpin + 'static,
        Sink_::Error: std::error::Error + Send + Sync + 'static,
        TryStream_: TryStream<Ok = Vec<u8>> + Send + Unpin + 'static,
        TryStream_::Error: std::error::Error + Send + Sync + 'static,
    {
        let (in_requests_sender, in_requests_receiver) = futures::channel::mpsc::channel(10);
        let (out_requests_sender, out_requests_receiver) = futures::channel::mpsc::channel(10);
        let (in_responses_sender, in_responses_receiver) = futures::channel::mpsc::channel(10);
        let (out_responses_sender, out_responses_receiver) = futures::channel::mpsc::channel(10);
        let client = Client::new(out_requests_sender, in_responses_receiver);
        let server_task = async_std::task::spawn(async move {
            super::server::run(&request_handler, in_requests_receiver, out_responses_sender).await
        });
        let packet_reader_task = async_std::task::spawn(async move {
            Self::read_packets(receive, in_requests_sender, in_responses_sender).await
        });
        let packet_sender_task = async_std::task::spawn(async move {
            let result = futures::stream::select(
                out_requests_receiver.map(Packet::Request),
                out_responses_receiver.map(Packet::Response),
            )
            .map(|packet| Ok(packet.build()))
            .forward(send)
            .await;
            result.map_err(anyhow::Error::from)
        });
        Self {
            client,
            server_task,
            packet_reader_task,
            packet_sender_task,
        }
    }

    pub fn new_client<Sink_, TryStream_>(send: Sink_, receive: TryStream_) -> Self
    where
        Sink_: Sink<Vec<u8>> + Send + Unpin + 'static,
        Sink_::Error: std::error::Error + Send + Sync + 'static,
        TryStream_: TryStream<Ok = Vec<u8>> + Send + Unpin + 'static,
        TryStream_::Error: std::error::Error + Send + Sync + 'static,
    {
        Self::new(send, receive, super::server::NoServer)
    }

    pub fn client(&mut self) -> &mut Client {
        &mut self.client
    }

    pub async fn join(self) -> anyhow::Result<()> {
        let Endpoint {
            packet_reader_task,
            packet_sender_task,
            server_task,
            ..
        } = self;
        futures::try_join!(packet_reader_task, packet_sender_task, server_task)?;
        Ok(())
    }

    // TODO use structured error or anyhow context
    async fn read_packets<Stream_>(
        receive: Stream_,
        mut request_sender: futures::channel::mpsc::Sender<Request>,
        mut response_sender: futures::channel::mpsc::Sender<Response>,
    ) -> anyhow::Result<()>
    where
        Stream_: TryStream<Ok = Vec<u8>> + Unpin,
        Stream_::Error: std::error::Error + Send + Sync + 'static,
    {
        let mut packet_stream = PacketStream::new(receive);
        loop {
            let next_item = packet_stream.try_next().await?;
            if let Some(packet) = next_item {
                match packet {
                    Packet::Request(request) => request_sender.send(request).await?,
                    Packet::Response(response) => response_sender.send(response).await?,
                }
            } else {
                tracing::debug!("end of endpoint stream");
                return Ok(());
            }
        }
    }
}
