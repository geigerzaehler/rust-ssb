use futures::prelude::*;
use std::pin::Pin;
use std::task::{Context, Poll};

/// Buffer for a fixed number of bytes to be read with [AsyncRead].
///
/// ```
/// let source_data = vec![1u8; 16];
/// let source = futures::io::Cursor::new(&source_data);
/// let read_buf = ReadBuffer::new(8);
/// let result = read_buf.poll_read(source)
/// assert_eq
///
///
///
/// ```
#[derive(Debug)]
pub struct ReadBuffer {
    data: Vec<u8>,
    read_count: usize,
}

impl ReadBuffer {
    pub fn new(size: usize) -> Self {
        ReadBuffer {
            data: vec![0u8; size],
            read_count: 0,
        }
    }

    // TODO document, test
    // Read bytes from `reader` into the buffer.
    pub fn poll_read(
        &mut self,
        mut reader: Pin<&mut impl AsyncRead>,
        cx: &mut Context,
    ) -> Poll<std::io::Result<Vec<u8>>> {
        loop {
            let buf = &mut self.data[self.read_count..];
            let read_count_current = futures::ready!(reader.as_mut().poll_read(cx, buf))?;
            if read_count_current == 0 {
                return Poll::Ready(Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof)));
            }

            self.read_count += read_count_current;
            if self.read_count == self.data.len() {
                return Poll::Ready(Ok(self.finish()));
            }
        }
    }

    fn finish(&mut self) -> Vec<u8> {
        self.read_count = 0;
        std::mem::replace(&mut self.data, Vec::new())
    }
}
