use bytes::Buf as _;
use futures::prelude::*;
use std::pin::Pin;
use std::task::{Context, Poll};

/// A [Sink] for `Vec<u8>` that encrypts data and sends it to the underlying `Writer`
#[pin_project::pin_project]
pub struct Encrypt<Writer: AsyncWrite> {
    #[pin]
    writer: Writer,
    params: crate::cipher::Params,
    /// Encrypted bytes to be written to the underlying `writer`.
    buffer: bytes::Bytes,
}

impl<Writer: AsyncWrite> Encrypt<Writer> {
    pub fn new(writer: Writer, params: crate::cipher::Params) -> Self {
        Encrypt {
            writer,
            params,
            buffer: bytes::Bytes::new(),
        }
    }

    fn poll_flush_buffer(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        let mut this = self.project();
        loop {
            if this.buffer.is_empty() {
                return Poll::Ready(Ok(()));
            }
            let written = futures::ready!(this.writer.as_mut().poll_write(cx, &*this.buffer))?;
            this.buffer.advance(written);
        }
    }
}

impl<Writer: AsyncWrite> Sink<Vec<u8>> for Encrypt<Writer> {
    type Error = std::io::Error;

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        futures::ready!(self.poll_flush_buffer(cx))?;
        Poll::Ready(Ok(()))
    }

    fn start_send(self: Pin<&mut Self>, data: Vec<u8>) -> Result<(), Self::Error> {
        debug_assert!(self.buffer.is_empty());
        let this = self.project();
        let mut buffer = bytes::BytesMut::new();
        this.params.encrypt(&mut buffer, &data);
        *this.buffer = buffer.freeze();
        Ok(())
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        futures::ready!(self.as_mut().poll_flush_buffer(cx))?;
        futures::ready!(self.project().writer.poll_flush(cx))?;
        Poll::Ready(Ok(()))
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        let this = self.as_mut().project();
        if this.buffer.is_empty() {
            let goodbye = this.params.goodbye();
            *this.buffer = bytes::Bytes::from(goodbye);
        }
        futures::ready!(self.as_mut().poll_flush_buffer(cx))?;
        futures::ready!(self.project().writer.poll_close(cx))?;
        Poll::Ready(Ok(()))
    }
}
