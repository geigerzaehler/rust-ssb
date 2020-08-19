use futures::prelude::*;
use std::pin::Pin;
use std::task::{Context, Poll};

/// Combine a [Sink] and a [Stream] into an object that implements both `Sink +
/// Stream`.
#[pin_project::pin_project]
#[derive(Debug)]
pub struct Duplex<R, W> {
    #[pin]
    reader: R,
    #[pin]
    writer: W,
}

impl<R, W> Duplex<R, W> {
    /// Create a new instance.
    pub fn new(reader: R, writer: W) -> Self {
        Self { reader, writer }
    }

    /// Decomposes a duplex into its components.
    pub fn into_inner(self) -> (R, W) {
        (self.reader, self.writer)
    }
}

impl<R: Stream, W> Stream for Duplex<R, W> {
    type Item = <R as Stream>::Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.project().reader.poll_next(cx)
    }
}

impl<Item, R, W: Sink<Item>> Sink<Item> for Duplex<R, W> {
    type Error = <W as Sink<Item>>::Error;

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.project().writer.poll_ready(cx)
    }

    fn start_send(self: Pin<&mut Self>, item: Item) -> Result<(), Self::Error> {
        self.project().writer.start_send(item)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.project().writer.poll_flush(cx)
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.project().writer.poll_close(cx)
    }
}
