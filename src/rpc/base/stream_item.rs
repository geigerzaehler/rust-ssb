use super::error::Error;
use super::packet::{Body, Request, Response};

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
