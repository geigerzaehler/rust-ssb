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
    Goodbye,
}

impl DecryptState {
    fn init() -> Self {
        DecryptState::ReadingHeader {
            buffer: ReadBuffer::new(vec![0u8; BOXED_HEADER_SIZE]),
        }
    }
}

impl<Reader: AsyncRead> Stream for Decrypt<Reader> {
    type Item = Result<Vec<u8>, DecryptError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        let result = futures::ready!(self.as_mut().poll_next_inner(cx));
        if let Some(Err(_)) = result {
            *self.project().state = DecryptState::Closed
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
                                buffer: ReadBuffer::new(vec![0u8; len as usize]),
                            }
                        }
                        None => {
                            *this.state = DecryptState::Goodbye;
                            return Poll::Ready(None);
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
                DecryptState::Goodbye => return Poll::Ready(None),
            };
        }
    }
}

struct ReadBuffer {
    data: Vec<u8>,
    read_count: usize,
}

impl ReadBuffer {
    fn new(data: Vec<u8>) -> Self {
        ReadBuffer {
            data,
            read_count: 0,
        }
    }

    fn poll_read(
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
