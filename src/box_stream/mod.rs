//! Stream encryption protocol

mod box_crypt;
mod decrypt;
mod encrypt;

use futures::prelude::*;

pub use box_crypt::BoxCrypt;
pub use decrypt::{Decrypt, DecryptError};
pub use encrypt::Encrypt;

/// Take a duplex stream and create a [Sink] for sending encrypted data and a [Stream] for
/// receiving and decrypting data.
pub fn box_stream<Stream: AsyncRead + AsyncWrite + Unpin>(
    stream: Stream,
    params: BoxStreamParams,
) -> (
    Encrypt<futures::io::WriteHalf<Stream>>,
    Decrypt<futures::io::ReadHalf<Stream>>,
) {
    let (raw_reader, raw_writer) = stream.split();
    (
        Encrypt::new(raw_writer, params.encrypt),
        Decrypt::new(raw_reader, params.decrypt),
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoxStreamParams {
    pub encrypt: BoxCrypt,
    pub decrypt: BoxCrypt,
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::test_utils::*;

    #[proptest]
    fn crypt_stream(messages: Vec<Vec<u8>>) {
        let _ = sodiumoxide::init();
        async_std::task::block_on(async move {
            let params = BoxCrypt::arbitrary();
            let (writer, reader) = async_pipe::pipe();
            let reader = Decrypt::new(reader, params.clone());
            let mut writer = Encrypt::new(writer, params.clone());

            let data = messages.concat();
            let write_handle = async_std::task::spawn(async move {
                for data in messages {
                    writer.send(data).await.unwrap();
                }
                writer.close().await.unwrap();
            });
            let data_read = reader.try_concat().await.unwrap();
            prop_assert_eq!(data_read, data);
            write_handle.await;
            Ok(())
        })?;
    }

    #[proptest]
    fn early_termination(
        #[strategy(proptest::collection::vec(any::<u8>(), 1..30))] data: Vec<u8>,
        cutoff: proptest::sample::Index,
    ) {
        let _ = sodiumoxide::init();
        async_std::task::block_on(async move {
            let params = BoxCrypt::arbitrary();
            let (raw_writer, raw_reader) = async_pipe::pipe();
            let cutoff = cutoff.index(data.len());
            let raw_reader = raw_reader.take(cutoff as u64);
            let reader = Decrypt::new(raw_reader, params.clone());
            let mut writer = Encrypt::new(raw_writer, params);

            async_std::task::spawn(async move {
                let _ = writer.send(data).await;
            });

            let items = reader.collect::<Vec<_>>().await;
            let err = items.last().unwrap().as_ref().unwrap_err();
            match err {
                DecryptError::Io(io_error) => {
                    prop_assert_eq!(io_error.kind(), std::io::ErrorKind::UnexpectedEof)
                }
                _ => prop_assert!(false),
            }
            Ok(())
        })?;
    }
}
