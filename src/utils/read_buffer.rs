use futures::prelude::*;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

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

    pub fn poll_read(
        &mut self,
        mut reader: Pin<&mut impl AsyncRead>,
        cx: &mut Context,
    ) -> Poll<io::Result<&[u8]>> {
        loop {
            let buf = &mut self.data[self.read_count..];
            let read_count_current = futures::ready!(reader.as_mut().poll_read(cx, buf))?;
            if read_count_current == 0 {
                return Poll::Ready(Err(io::Error::from(io::ErrorKind::UnexpectedEof)));
            }

            self.read_count += read_count_current;
            if self.read_count == self.data.len() {
                return Poll::Ready(Ok(&self.data));
            }
        }
    }
}
