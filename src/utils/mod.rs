use futures::prelude::*;

mod read_buffer;
#[doc(inline)]
pub use read_buffer::ReadBuffer;

mod oneshot;
#[doc(inline)]
pub use oneshot::{OneshotClosed, OneshotSink, OneshotStream};

/// Convert [AsyncRead] into a [Stream]. Polling the resulting stream will poll
/// the reader for 4096 bytes and return a [Vec] of all the bytes that were read.
pub fn read_to_stream(
    read: impl AsyncRead + Unpin,
) -> impl Stream<Item = Result<Vec<u8>, std::io::Error>> {
    const BUF_SIZE: usize = 4096;
    let mut read = read;
    let mut buf = vec![0u8; BUF_SIZE];
    futures::stream::poll_fn(move |cx| {
        let result = match futures::ready!(std::pin::Pin::new(&mut read).poll_read(cx, &mut buf)) {
            Ok(size) => {
                if size == 0 {
                    None
                } else {
                    Some(Ok(Vec::from(&buf[..size])))
                }
            }
            Err(err) => Some(Err(err)),
        };
        std::task::Poll::Ready(result)
    })
}
