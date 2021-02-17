use crate::error::Error;
use crate::packet::{Body, Request, Response};

// TODO doc
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub enum StreamMessage {
    Data(Body),
    Error(Error),
    End,
}

impl StreamMessage {
    pub(crate) fn into_response(self, number: u32) -> Response {
        Response::Stream {
            number,
            message: self,
        }
    }

    pub(crate) fn into_request(self, number: u32) -> Request {
        Request::Stream {
            number,
            message: self,
        }
    }

    /// Returns true if the message ends the stream.
    ///
    /// That is if the message is [StreamMessage::Error] or [StreamMessage::End].
    pub fn is_end(&self) -> bool {
        match self {
            StreamMessage::Data(_) => false,
            StreamMessage::Error(_) => true,
            StreamMessage::End => true,
        }
    }
}
