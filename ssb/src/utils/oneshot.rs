//! Provide wrappers for [`futures::channel::oneshot::Sender`] and
//! [`futures::channel::oneshot::Receiver`] that that implement [`Stream`] and [`Sink`]
use futures::prelude::*;
use std::{pin::Pin, task::Poll};

/// Error returned from sending to [`OneshotSink`] when the corresponding receiver was dropped.
#[derive(Debug, PartialEq, Eq)]
pub struct OneshotClosed;

#[derive(Debug)]
pub struct OneshotSink<T> {
    sender: Option<futures::channel::oneshot::Sender<T>>,
}

impl<T> OneshotSink<T> {
    pub fn new(sender: futures::channel::oneshot::Sender<T>) -> Self {
        Self {
            sender: Some(sender),
        }
    }
}

/// Wrap [`futures::channel::oneshot::Sender`] to implement [`Sink`]
///
/// Sending a value to the sink is the same as sending it to the original
/// [`futures::channel::oneshot::Sender`].
///
/// ```rust
/// # use ssb::utils::{OneshotSink, OneshotClosed};
/// # use futures::prelude::*;
/// # async_std::task::block_on(async {
/// let (sender, receiver) = futures::channel::oneshot::channel::<()>();
/// let mut sink = OneshotSink::new(sender);
/// sink.send(()).await;
/// let _ = sink.send(()).await;
/// assert_eq!(receiver.await, Ok(()));
/// # });
/// ```
///
/// If the receiver is dropped the, sink returns an error.
///
/// ```rust
/// # use ssb::utils::{OneshotSink, OneshotClosed};
/// # use futures::prelude::*;
/// # async_std::task::block_on(async {
/// let (sender, receiver) = futures::channel::oneshot::channel::<()>();
/// let mut sink = OneshotSink::new(sender);
/// drop(receiver);
/// let result = sink.send(()).await;
/// assert_eq!(result, Err(OneshotClosed));
/// # });
/// ```
impl<T> Sink<T> for OneshotSink<T> {
    type Error = OneshotClosed;

    fn poll_ready(
        self: Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        if self.sender.is_some() {
            Poll::Ready(Ok(()))
        } else {
            Poll::Ready(Err(OneshotClosed))
        }
    }

    fn start_send(self: Pin<&mut Self>, item: T) -> Result<(), Self::Error> {
        if let Some(sender) = self.get_mut().sender.take() {
            let result = sender.send(item);
            if result.is_ok() {
                Ok(())
            } else {
                Err(OneshotClosed)
            }
        } else {
            Err(OneshotClosed)
        }
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        if self.sender.is_some() {
            Poll::Ready(Ok(()))
        } else {
            Poll::Ready(Err(OneshotClosed))
        }
    }

    fn poll_close(
        self: Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        self.get_mut().sender.take();
        Poll::Ready(Ok(()))
    }
}

/// Wrap [`futures::channel::oneshot::Receiver`] to implement [`Stream`]
///
/// If a value is sent to the channel that value is emitted from the stream, then the stream ends
/// immediately.
///
/// ```rust
/// # use ssb::utils::OneshotStream;
/// # use futures::prelude::*;
/// # async_std::task::block_on(async {
/// let (sender, receiver) = futures::channel::oneshot::channel::<()>();
/// let mut stream = OneshotStream::new(receiver);
/// sender.send(());
/// assert_eq!(stream.next().await, Some(()));
/// assert_eq!(stream.next().await, None);
/// # });
/// ```
///
/// If the sender is dropped the, stream ends
///
/// ```rust
/// # use ssb::utils::OneshotStream;
/// # use futures::prelude::*;
/// # async_std::task::block_on(async {
/// let (sender, receiver) = futures::channel::oneshot::channel::<()>();
/// let mut stream = OneshotStream::new(receiver);
/// drop(sender);
/// assert_eq!(stream.next().await, None);
/// # });
/// ```
#[derive(Debug)]
pub struct OneshotStream<T> {
    receiver: Option<futures::channel::oneshot::Receiver<T>>,
}

impl<T> OneshotStream<T> {
    pub fn new(receiver: futures::channel::oneshot::Receiver<T>) -> Self {
        Self {
            receiver: Some(receiver),
        }
    }
}

impl<T: std::fmt::Debug> Stream for OneshotStream<T> {
    type Item = T;

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        let opt_receiver = &mut self.get_mut().receiver;
        if let Some(receiver) = opt_receiver {
            let result = futures::ready!(receiver.poll_unpin(cx));
            opt_receiver.take();
            match result {
                Ok(value) => Poll::Ready(Some(value)),
                Err(_cancelled) => Poll::Ready(None),
            }
        } else {
            Poll::Ready(None)
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[async_std::test]
    async fn stream_send() {
        let (sender, receiver) = futures::channel::oneshot::channel::<()>();
        let mut stream = OneshotStream::new(receiver);
        let handle = async_std::task::spawn(async move {
            assert_eq!(stream.next().await, Some(()));
            assert_eq!(stream.next().await, None);
        });
        async_std::task::sleep(std::time::Duration::from_millis(5)).await;
        let _ = sender.send(());
        handle.await;
    }

    #[async_std::test]
    async fn stream_drop_sender_end() {
        let (sender, receiver) = futures::channel::oneshot::channel::<()>();
        let mut stream = OneshotStream::new(receiver);
        let handle = async_std::task::spawn(async move { stream.next().await });
        async_std::task::sleep(std::time::Duration::from_millis(5)).await;
        drop(sender);
        assert_eq!(handle.await, None);
    }
}
