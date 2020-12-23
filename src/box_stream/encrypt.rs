use bytes::{Buf as _, Bytes};
use futures::prelude::*;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use super::box_crypt::{BoxCrypt, Packet};

#[pin_project::pin_project]
pub struct Encrypt<Writer> {
    #[pin]
    writer: Writer,
    params: BoxCrypt,
    buffer: Bytes,
}

impl<Writer: AsyncWrite> Encrypt<Writer> {
    pub fn new(writer: Writer, params: BoxCrypt) -> Self {
        Encrypt {
            writer,
            params,
            buffer: Bytes::new(),
        }
    }

    fn poll_flush_buffer(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), io::Error>> {
        let mut this = self.project();
        loop {
            let written = futures::ready!(this.writer.as_mut().poll_write(cx, &*this.buffer))?;
            this.buffer.advance(written);
            if this.buffer.is_empty() {
                return Poll::Ready(Ok(()));
            }
        }
    }
}

impl<Writer: AsyncWrite> Sink<Vec<u8>> for Encrypt<Writer> {
    type Error = io::Error;

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        futures::ready!(self.poll_flush_buffer(cx))?;
        Poll::Ready(Ok(()))
    }

    fn start_send(self: Pin<&mut Self>, data: Vec<u8>) -> Result<(), Self::Error> {
        debug_assert!(self.buffer.is_empty());
        let this = self.project();
        let mut boxed_data = Vec::new();
        for packet in Packet::build(&data) {
            boxed_data.extend_from_slice(&this.params.encrypt(packet));
        }

        *this.buffer = Bytes::from(boxed_data);
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
            *this.buffer = Bytes::from(goodbye);
        }
        futures::ready!(self.as_mut().poll_flush_buffer(cx))?;
        futures::ready!(self.project().writer.poll_close(cx))?;
        Poll::Ready(Ok(()))
    }
}
