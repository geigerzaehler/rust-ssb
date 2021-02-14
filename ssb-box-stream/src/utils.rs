use futures::prelude::*;
use std::pin::Pin;
use std::task::{Context, Poll};

/// Buffer for a fixed number of bytes to be read with [AsyncRead].
#[derive(Debug)]
pub struct ReadBuffer {
    data: Vec<u8>,
    /// Number of bytes that have been written to `data`.
    filled: usize,
}

impl ReadBuffer {
    /// Panics if `size` is 0.
    pub fn new(size: usize) -> Self {
        assert!(size > 0);
        ReadBuffer {
            data: vec![0u8; size],
            filled: 0,
        }
    }

    /// Poll to read data from `source` into `self`.
    ///
    /// Calls [`AsyncRead::poll_read`] to fill `self`. If `self` has been filled up to its size all
    /// data is returned and `self` is reset to its initial state.
    ///
    /// If `source` ends and there is still data to be read then an error of kind
    /// [`std::io::ErrorKind::UnexpectedEof`] is returned.
    pub fn poll_read(
        &mut self,
        cx: &mut Context,
        mut reader: Pin<&mut impl AsyncRead>,
    ) -> Poll<std::io::Result<Vec<u8>>> {
        loop {
            let buf = &mut self.data[self.filled..];
            let read_count_current = futures::ready!(reader.as_mut().poll_read(cx, buf))?;
            if read_count_current == 0 {
                return Poll::Ready(Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof)));
            }

            self.filled += read_count_current;
            if self.filled == self.data.len() {
                return Poll::Ready(Ok(self.finish()));
            }
        }
    }

    fn finish(&mut self) -> Vec<u8> {
        self.filled = 0;
        std::mem::replace(&mut self.data, Vec::new())
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use proptest::prelude::*;

    #[test_strategy::proptest]
    fn poll_read_ok(source_data: Vec<u8>, read_size_index: proptest::sample::Index) {
        prop_assume!(!source_data.is_empty());
        let mut source = futures::io::Cursor::new(&source_data);
        let read_size = read_size_index.index(source_data.len());
        prop_assume!(read_size > 0);
        let mut read_buf = ReadBuffer::new(read_size);
        let read = async_std::task::block_on({
            let source = &mut source;
            futures::future::poll_fn(move |cx| read_buf.poll_read(cx, Pin::new(source)))
        })
        .unwrap();

        prop_assert_eq!(source.position() as usize, read_size);
        prop_assert_eq!(&read, &source_data[..read_size]);
    }

    #[test_strategy::proptest]
    fn poll_read_eof(source_data: Vec<u8>) {
        prop_assume!(!source_data.is_empty());
        let mut source = futures::io::Cursor::new(&source_data);
        let mut read_buf = ReadBuffer::new(source_data.len() + 1);
        let err = async_std::task::block_on({
            let source = &mut source;
            futures::future::poll_fn(move |cx| read_buf.poll_read(cx, Pin::new(source)))
        })
        .unwrap_err();

        prop_assert_eq!(err.kind(), std::io::ErrorKind::UnexpectedEof);
    }
}
