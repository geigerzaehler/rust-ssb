use futures::prelude::*;
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::utils::ReadBuffer;

/// A [Stream] of `Vec<u8>` that decrypts and authenticates data from the underlying `Reader`.
#[pin_project::pin_project]
pub struct Decrypt<Reader: AsyncRead> {
    #[pin]
    reader: Reader,
    params: crate::cipher::Params,
    state: DecryptState,
}

impl<Reader: AsyncRead> Decrypt<Reader> {
    pub fn new(reader: Reader, params: crate::cipher::Params) -> Self {
        Decrypt {
            reader,
            params,
            state: DecryptState::init(),
        }
    }
}

/// Error when decrypting and authenticating data.
#[derive(thiserror::Error, Debug)]
pub enum DecryptError {
    /// The underlying source errored.
    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// Failed to decrypt and authenticate packet body
    #[error("Failed to decrypt and authenticate packet body")]
    UnboxBody,

    /// Failed to decrypt and authenticate packet header
    #[error("Failed to decrypt and authenticate packet header")]
    UnboxHeader,

    /// Received packet that exceeds maximum packet size
    #[error("Received packet that exceeds maximum packet size")]
    ExceededMaxPacketSize,
}

enum DecryptState {
    Closed,
    ReadingHeader {
        buffer: ReadBuffer,
    },
    ReadingBody {
        auth_tag: sodiumoxide::crypto::secretbox::Tag,
        buffer: ReadBuffer,
    },
}

impl DecryptState {
    fn init() -> Self {
        DecryptState::ReadingHeader {
            buffer: ReadBuffer::new(crate::cipher::BOXED_HEADER_SIZE),
        }
    }
}

impl<Reader: AsyncRead> Stream for Decrypt<Reader> {
    type Item = Result<Vec<u8>, DecryptError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        let result = futures::ready!(self.as_mut().poll_next_inner(cx));
        match result {
            Some(Err(_)) | None => *self.project().state = DecryptState::Closed,
            _ => (),
        }
        Poll::Ready(result)
    }
}

impl<Reader: AsyncRead> Decrypt<Reader> {
    fn poll_next_inner(
        mut self: Pin<&mut Self>,
        cx: &mut Context,
    ) -> Poll<Option<Result<Vec<u8>, DecryptError>>> {
        loop {
            let mut this = self.as_mut().project();
            match &mut this.state {
                DecryptState::Closed => return Poll::Ready(None),
                DecryptState::ReadingHeader { buffer } => {
                    let boxed_header = futures::ready!(buffer.poll_read(cx, this.reader))?;
                    let mut boxed_header_array = [0u8; crate::cipher::BOXED_HEADER_SIZE];
                    boxed_header_array.copy_from_slice(&boxed_header);
                    match this
                        .params
                        .decrypt_header(&boxed_header_array)
                        .map_err(|()| DecryptError::UnboxHeader)?
                    {
                        Some((len, auth_tag)) => {
                            if len >= crate::cipher::MAX_PACKET_SIZE_BYTES {
                                *this.state = DecryptState::Closed;
                                return Poll::Ready(Some(Err(DecryptError::ExceededMaxPacketSize)));
                            }
                            *this.state = DecryptState::ReadingBody {
                                auth_tag,
                                buffer: ReadBuffer::new(len as usize),
                            }
                        }
                        None => {
                            *this.state = DecryptState::Closed;
                        }
                    }
                }
                DecryptState::ReadingBody { auth_tag, buffer } => {
                    let boxed_body = futures::ready!(buffer.poll_read(cx, this.reader))?;
                    let body = this
                        .params
                        .decrypt_body(auth_tag, &boxed_body)
                        .map_err(|()| DecryptError::UnboxBody)?;
                    *this.state = DecryptState::init();
                    return Poll::Ready(Some(Ok(body)));
                }
            };
        }
    }
}
