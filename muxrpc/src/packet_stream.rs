//! Provides [PacketStream] for parsing RPC packets from a byte stream.

use bytes::BufMut as _;
use futures::prelude::*;
use std::pin::Pin;
use std::task::{Context, Poll};

use super::packet::{Header, HeaderParseError, Packet, PacketParseError};

#[derive(Debug, thiserror::Error)]
/// Error receiving an RPC [Packet].
pub enum NextPacketError {
    #[error("Failed to read bytes")]
    Source(#[source] Box<dyn std::error::Error + Send + Sync + 'static>),
    #[error("Failed to parse packet header")]
    InvalidHeader(
        #[source]
        #[from]
        HeaderParseError,
    ),
    #[error("Failed to parse packet")]
    PacketParse(
        #[source]
        #[from]
        PacketParseError,
    ),
    #[error("Unexpected end of stream while parsing packet")]
    UnexpectedEndOfStream,
}

#[pin_project::pin_project]
#[derive(Debug)]
/// [Stream] of [Packet]s parsed from underlying [Stream] of bytes.
pub struct PacketStream<Stream> {
    #[pin]
    stream: Stream,
    reader: PacketReader,
    buffer: bytes::Bytes,
}

impl<Stream> PacketStream<Stream> {
    pub fn new(stream: Stream) -> Self {
        Self {
            stream,
            reader: PacketReader::new(),
            buffer: bytes::Bytes::new(),
        }
    }
}

impl<Stream_> Stream for PacketStream<Stream_>
where
    Stream_: TryStream<Ok = Vec<u8>>,
    Stream_::Error: std::error::Error + Send + Sync + 'static,
{
    type Item = Result<Packet, NextPacketError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        loop {
            let mut this = self.as_mut().project();

            if this.buffer.is_empty() {
                match futures::ready!(this.stream.try_poll_next(cx)) {
                    Some(Ok(data)) => *this.buffer = bytes::Bytes::from(data),
                    Some(Err(err)) => {
                        return Poll::Ready(Some(Err(NextPacketError::Source(Box::new(err)))))
                    }
                    None => {
                        if this.reader.is_empty() {
                            return Poll::Ready(None);
                        } else {
                            return Poll::Ready(Some(Err(NextPacketError::UnexpectedEndOfStream)));
                        }
                    }
                };
            }

            if let Some(packet_result) = this.reader.put(&mut this.buffer) {
                return Poll::Ready(packet_result.transpose());
            }
        }
    }
}

/// Buffer that is fed bytes until it produces [Packet].
///
/// Call [PacketReader::put] repeatedly until a [Packet] or an error is returned.
#[derive(Debug)]
enum PacketReader {
    ReadingHeader {
        buffer: bytes::BytesMut,
    },
    ReadingBody {
        header: Header,
        buffer: bytes::BytesMut,
    },
}

impl PacketReader {
    fn new() -> Self {
        Self::ReadingHeader {
            buffer: bytes::BytesMut::with_capacity(Header::SIZE),
        }
    }

    fn put(
        &mut self,
        mut data: impl bytes::Buf,
    ) -> Option<Result<Option<Packet>, NextPacketError>> {
        loop {
            if !data.has_remaining() {
                return None;
            }

            match self {
                Self::ReadingHeader { buffer } => {
                    use std::convert::TryInto as _;
                    let header_data = buffer.put(&mut data);
                    // .try_into() is guaranteed to not fail since the buffer
                    // holds exactly Header::SIZE bytes.
                    let header_data = header_data.as_slice().try_into().unwrap();
                    let header = match Header::parse(header_data) {
                        Ok(Some(header)) => header,
                        Ok(None) => {
                            return Some(Ok(None));
                        }
                        Err(err) => return Some(Err(NextPacketError::InvalidHeader(err))),
                    };
                    if header.body_len == 0 {
                        *self = Self::new();
                        return match Packet::parse(header, Vec::new()) {
                            Ok(packet) => Some(Ok(Some(packet))),
                            Err(err) => Some(Err(NextPacketError::PacketParse(err))),
                        };
                    }

                    *self = Self::ReadingBody {
                        header,
                        buffer: bytes::BytesMut::with_capacity(header.body_len as usize),
                    };
                }
                Self::ReadingBody { header, buffer } => {
                    let body_data = buffer.put(&mut data);
                    let packet_result = match Packet::parse(*header, body_data) {
                        Ok(packet) => Ok(Some(packet)),
                        Err(err) => Err(NextPacketError::PacketParse(err)),
                    };
                    *self = Self::new();
                    return Some(packet_result);
                }
            }
        }
    }

    fn is_empty(&self) -> bool {
        match self {
            PacketReader::ReadingHeader { buffer } => buffer.is_empty(),
            PacketReader::ReadingBody { .. } => false,
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use proptest::prelude::*;

    #[test_strategy::proptest]
    fn read_packets(
        #[strategy(proptest::collection::vec(any::<Packet>(), 0..10))] packets: Vec<Packet>,
        chunks: proptest::sample::Index,
    ) {
        async_std::task::block_on(async move {
            use std::convert::Infallible;
            let packets2 = packets.clone();
            let packet_data = packets
                .into_iter()
                .map(|packet| packet.build())
                // Insert the "goodbye" header and some garbage
                .chain(vec![vec![0u8; Header::SIZE], vec![1u8; Header::SIZE]])
                .collect::<Vec<Vec<u8>>>()
                .concat();
            let chunks = chunks.index(packet_data.len());
            prop_assume!(chunks > 0);
            let chunk_size = packet_data.len() / chunks;
            let packet_data_source = futures::stream::iter(packet_data.chunks(chunk_size))
                .map(|chunk| -> Result<Vec<u8>, Infallible> { Ok(chunk.to_vec()) });
            let mut packet_stream =
                PacketStream::new(packet_data_source).map_err(|e| panic!("{:?}", e));
            let mut packets_received = Vec::new();
            packets_received.send_all(&mut packet_stream).await.unwrap();
            prop_assert_eq!(packets_received.len(), packets2.len());
            prop_assert_eq!(packets_received, packets2);
            Ok(())
        })?;
    }

    #[async_std::test]
    async fn unexpected_end_of_stream() {
        let packet_data = vec![1u8; Header::SIZE];
        let packet_data_source = futures::stream::once(async move { packet_data })
            .map(Result::<_, std::convert::Infallible>::Ok);
        let result = PacketStream::new(packet_data_source)
            .try_for_each(|_| async { Ok(()) })
            .await;
        match result.unwrap_err() {
            NextPacketError::UnexpectedEndOfStream => (),
            e => panic!("Unexpected error {:?}", e),
        }
    }
}
