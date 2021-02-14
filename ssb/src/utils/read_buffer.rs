use futures::prelude::*;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

/// Buffer for a fixed number of bytes to be read.
///
/// The buffer can be filled with an [AsyncRead] using [ReadBuffer::poll_read] and a [bytes::Buf]
/// using [ReadBuffer::put]. Once the expected number of bytes are read the buffer data is returned
/// and the buffer is reset.
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

    // Read bytes from `reader` into the buffer.
    pub fn poll_read(
        &mut self,
        mut reader: Pin<&mut impl AsyncRead>,
        cx: &mut Context,
    ) -> Poll<io::Result<Vec<u8>>> {
        loop {
            let buf = &mut self.data[self.read_count..];
            let read_count_current = futures::ready!(reader.as_mut().poll_read(cx, buf))?;
            if read_count_current == 0 {
                return Poll::Ready(Err(io::Error::from(io::ErrorKind::UnexpectedEof)));
            }

            self.read_count += read_count_current;
            if self.read_count == self.data.len() {
                return Poll::Ready(Ok(self.finish()));
            }
        }
    }

    pub fn put(&mut self, data: &mut impl bytes::Buf) -> Option<Vec<u8>> {
        let need = self.data.len() - self.read_count;
        let read_count_current = std::cmp::min(data.remaining(), need);
        let end = self.read_count + read_count_current;
        data.copy_to_slice(&mut self.data[self.read_count..end]);
        self.read_count += read_count_current;
        if self.read_count == self.data.len() {
            Some(self.finish())
        } else {
            None
        }
    }

    pub fn is_empty(&self) -> bool {
        self.read_count == 0
    }

    fn finish(&mut self) -> Vec<u8> {
        self.read_count = 0;
        std::mem::replace(&mut self.data, Vec::new())
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use bytes::Buf as _;
    use proptest::prelude::*;

    #[test_strategy::proptest]
    fn put_twice(
        #[strategy(1..30usize)] input_size: usize,
        buffer_size: proptest::sample::Index,
        input_split: proptest::sample::Index,
    ) {
        let data = vec![1u8; input_size];

        let buffer_size = buffer_size.index(input_size + 1);
        prop_assume!(buffer_size > 0);

        let mut buffer = ReadBuffer::new(buffer_size);
        let expected_content = Vec::from(&data[..buffer_size]);

        let mut input1 = bytes::Bytes::from(data);
        let mut input2 = input1.split_off(input_split.index(buffer_size));

        let output = buffer.put(&mut input1);
        prop_assert_eq!(output, None);
        prop_assert!(!input1.has_remaining());

        let output = buffer.put(&mut input2);
        prop_assert_eq!(output, Some(expected_content));
        prop_assert_eq!(input2.remaining(), input_size - buffer_size);
    }
}
