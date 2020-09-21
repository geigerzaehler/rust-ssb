use super::packet::{Body, Response};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Error {
    pub name: String,
    pub message: String,
}

#[derive(Debug)]
pub enum StreamItem {
    Data(Body),
    Error(Error),
    End,
}

impl StreamItem {
    pub(super) fn into_response(self, number: u32) -> Response {
        match self {
            StreamItem::Data(body) => Response::StreamData { number, body },
            StreamItem::Error(Error { name, message }) => Response::StreamError {
                number,
                name,
                message,
            },
            StreamItem::End => Response::StreamEnd { number },
        }
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
