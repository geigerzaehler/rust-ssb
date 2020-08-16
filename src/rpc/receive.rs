use futures::prelude::*;
use std::pin::Pin;
use std::task::{Context, Poll};

use super::packet::{Header, HeaderParseError, Packet, PacketParseError};
use crate::utils::ReadBuffer;

#[derive(Debug, thiserror::Error)]
/// Error receiving an RPC [Packet].
pub enum NextPacketError {
    #[error("Source")]
    Source(#[source] Box<dyn std::error::Error>),
    #[error("Invalid header")]
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

impl<Stream_, Error> Stream for PacketStream<Stream_>
where
    Stream_: Stream<Item = Result<Vec<u8>, Error>>,
    Error: std::error::Error + 'static,
{
    type Item = Result<Packet, NextPacketError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        loop {
            let mut this = self.as_mut().project();

            if this.buffer.is_empty() {
                match futures::ready!(this.stream.poll_next(cx)) {
                    Some(Ok(data)) => *this.buffer = bytes::Bytes::from(data),
                    Some(Err(err)) => {
                        return Poll::Ready(Some(Err(NextPacketError::Source(Box::new(err)))))
                    }
                    None => return Poll::Ready(None),
                };
            }

            if let Some(packet_result) = this.reader.put(&mut this.buffer) {
                return Poll::Ready(Some(packet_result));
            }
        }
    }
}

#[derive(Debug)]
/// Buffer that is fed bytes until it produces [Packet].
///
/// Call [PacketReader::put] repeatedly until a [Packet] or an error is returned.
enum PacketReader {
    ReadingHeader { buffer: ReadBuffer },
    ReadingBody { header: Header, buffer: ReadBuffer },
}

impl PacketReader {
    fn new() -> Self {
        Self::ReadingHeader {
            buffer: ReadBuffer::new(Header::SIZE),
        }
    }

    fn put(&mut self, mut data: impl bytes::Buf) -> Option<Result<Packet, NextPacketError>> {
        loop {
            if !data.has_remaining() {
                return None;
            }

            match self {
                Self::ReadingHeader { buffer } => {
                    use std::convert::TryInto as _;
                    let header_data = buffer.put(&mut data)?;
                    // .try_into() is guaranteed to not fail since we the buffer holds exactly
                    // Header::SIZE bytes.
                    let header_data = header_data.as_slice().try_into().unwrap();
                    let header = match Header::parse(header_data) {
                        Ok(header) => header,
                        Err(err) => return Some(Err(NextPacketError::InvalidHeader(err))),
                    };
                    *self = Self::ReadingBody {
                        header,
                        buffer: ReadBuffer::new(header.body_len as usize),
                    };
                }
                Self::ReadingBody { header, buffer } => {
                    let body_data = buffer.put(&mut data)?;
                    let packet_result =
                        Packet::new(*header, body_data).map_err(NextPacketError::PacketParse);
                    *self = Self::new();
                    return Some(packet_result);
                }
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::test_utils::*;

    #[proptest]
    fn read_packets(
        #[strategy(proptest::collection::vec(any::<Packet>(), 1..10))] packets: Vec<Packet>,
        chunks: proptest::sample::Index,
    ) {
        async_std::task::block_on(async move {
            use std::convert::Infallible;
            let packets2 = packets.clone();
            let packet_data = packets
                .into_iter()
                .map(|packet| packet.build())
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
}
