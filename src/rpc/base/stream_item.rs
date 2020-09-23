use super::error::Error;
#[cfg(test)]
use super::packet::Request;
use super::packet::{Body, Response};

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub enum StreamItem {
    Data(Body),
    Error(Error),
    End,
}

impl StreamItem {
    pub(super) fn into_response(self, number: u32) -> Response {
        Response::StreamItem { number, item: self }
    }

    #[cfg(test)]
    pub(super) fn into_request(self, number: u32) -> Request {
        Request::StreamItem { number, item: self }
    }

    pub fn is_end(&self) -> bool {
        match self {
            StreamItem::Data(_) => false,
            StreamItem::Error(_) => true,
            StreamItem::End => true,
        }
    }
}

impl From<Option<Result<Body, Error>>> for StreamItem {
    fn from(item: Option<Result<Body, Error>>) -> Self {
        match item {
            Some(Ok(body)) => Self::Data(body),
            Some(Err(error)) => Self::Error(error),
            None => Self::End,
        }
    }
}
