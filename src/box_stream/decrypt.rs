use crate::utils::ReadBuffer;

use futures::prelude::*;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use super::box_crypt::{BoxCrypt, BOXED_HEADER_SIZE};
use crate::crypto;

#[derive(thiserror::Error, Debug)]
pub enum DecryptError {
    #[error("IO error")]
    Io(
        #[from]
        #[source]
        io::Error,
    ),
    #[error("Failed to decrypt and authenticate packet body")]
    UnboxBody,
    #[error("Failed to decrypt and authenticate packet heder")]
    UnboxHeader,
}

#[pin_project::pin_project]
pub struct Decrypt<Reader> {
    #[pin]
    reader: Reader,
    params: BoxCrypt,
    state: DecryptState,
}

impl<Reader> Decrypt<Reader> {
    pub fn new(reader: Reader, params: BoxCrypt) -> Self {
        Decrypt {
            reader,
            params,
            state: DecryptState::init(),
        }
    }
}

enum DecryptState {
    Closed,
    ReadingHeader {
        buffer: ReadBuffer,
    },
    ReadingBody {
        auth_tag: crypto::secretbox::Tag,
        buffer: ReadBuffer,
    },
}

impl DecryptState {
    fn init() -> Self {
        DecryptState::ReadingHeader {
            buffer: ReadBuffer::new(BOXED_HEADER_SIZE),
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
                    let boxed_header = futures::ready!(buffer.poll_read(this.reader, cx))?;
                    let mut boxed_header_array = [0u8; BOXED_HEADER_SIZE];
                    boxed_header_array.copy_from_slice(boxed_header);
                    match this
                        .params
                        .decrypt_head(&boxed_header_array)
                        .map_err(|()| DecryptError::UnboxHeader)?
                    {
                        Some((len, auth_tag)) => {
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
                    let boxed_body = futures::ready!(buffer.poll_read(this.reader, cx))?;
                    let body = this
                        .params
                        .decrypt_body(auth_tag, boxed_body)
                        .map_err(|()| DecryptError::UnboxBody)?;
                    *this.state = DecryptState::init();
                    return Poll::Ready(Some(Ok(body)));
                }
            };
        }
    }
}
