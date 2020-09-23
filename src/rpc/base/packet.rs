use super::header::BodyType;

use super::error::Error;
pub use super::header::{Header, HeaderFlags, HeaderParseError};
use super::stream_item::StreamItem;

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[cfg_attr(test, proptest(no_params))]
pub enum Packet {
    Request(Request),
    Response(Response),
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub enum Request {
    Async {
        #[cfg_attr(test, proptest(strategy = "1..(u32::MAX / 2)"))]
        number: u32,
        #[cfg_attr(
            test,
            proptest(
                strategy = "proptest::collection::vec(proptest::arbitrary::any::<String>(), 1..3)"
            )
        )]
        method: Vec<String>,
        #[cfg_attr(test, proptest(value = "vec![]"))]
        args: Vec<serde_json::Value>,
    },
    StreamItem {
        #[cfg_attr(test, proptest(strategy = "1..(u32::MAX / 2)"))]
        number: u32,
        item: StreamItem,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[cfg_attr(test, proptest(no_params))]
pub enum Response {
    AsyncOk {
        #[cfg_attr(test, proptest(strategy = "1..(u32::MAX / 2)"))]
        number: u32,
        body: Body,
    },
    AsyncErr {
        #[cfg_attr(test, proptest(strategy = "1..(u32::MAX / 2)"))]
        number: u32,
        name: String,
        message: String,
    },
    StreamItem {
        #[cfg_attr(test, proptest(strategy = "1..(u32::MAX / 2)"))]
        number: u32,
        item: StreamItem,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum PacketParseError {
    #[error("Failed to decode JSON request body")]
    RequestBody {
        body: String,
        #[source]
        error: serde_json::Error,
    },
    #[error("Failed to decode error response body")]
    ErrorResponseBody {
        body: String,
        #[source]
        error: serde_json::Error,
    },
    #[error("Invalid string payload")]
    StringPlayloadEncoding {
        #[source]
        error: std::string::FromUtf8Error,
    },
    #[error("Unexpected body type {actual:?}. Expected {expected:?}")]
    UnexpectedBodyType {
        actual: BodyType,
        expected: BodyType,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
struct RequestBody {
    name: Vec<String>,
    // TODO generate json values
    #[cfg_attr(test, proptest(value = "vec![]"))]
    args: Vec<serde_json::Value>,
}

impl Packet {
    pub fn parse(header: Header, body: Vec<u8>) -> Result<Self, PacketParseError> {
        let request_number = header.request_number;
        let body = Body::parse(header.body_type, body)?;
        #[allow(clippy::collapsible_if)]
        let packet = if request_number > 0 {
            let number = request_number as u32;
            let request = if header.flags.is_stream {
                let item = parse_stream_item(&header.flags, body)?;
                Request::StreamItem { number, item }
            } else {
                // We are ignoring `header.flags.is_end_or_error`. It should
                // always be set to `false` since `true` for async requests is
                // unspecified.
                let json = body.into_json()?;
                let RequestBody { name, args } =
                    serde_json::from_slice(&json).map_err(|error| {
                        PacketParseError::RequestBody {
                            body: String::from_utf8_lossy(&json).into_owned(),
                            error,
                        }
                    })?;
                Request::Async {
                    number: header.request_number as u32,
                    method: name,
                    args,
                }
            };
            Packet::Request(request)
        } else {
            let number = -request_number as u32;
            let response = if header.flags.is_stream {
                let item = parse_stream_item(&header.flags, body)?;
                Response::StreamItem { number, item }
            } else {
                if header.flags.is_end_or_error {
                    let json = body.into_json()?;
                    let error = parse_error_json(&json)?;
                    Response::AsyncErr {
                        number,
                        name: error.name,
                        message: error.message,
                    }
                } else {
                    Response::AsyncOk { number, body }
                }
            };
            Packet::Response(response)
        };
        Ok(packet)
    }

    fn build_raw(self) -> RawPacket {
        match self {
            Packet::Request(request) => match request {
                Request::Async {
                    number,
                    method,
                    args,
                } => RawPacket {
                    request_number: number as i32,
                    is_stream: false,
                    is_end_or_error: false,
                    body: Body::json(&RequestBody { name: method, args }),
                },
                Request::StreamItem { number, item } => {
                    RawPacket::from_stream_item(number as i32, item)
                }
            },
            Packet::Response(response) => match response {
                Response::AsyncOk { number, body } => RawPacket {
                    request_number: -(number as i32),
                    is_stream: false,
                    is_end_or_error: false,
                    body,
                },
                Response::AsyncErr {
                    number,
                    name,
                    message,
                } => RawPacket {
                    request_number: -(number as i32),
                    is_stream: false,
                    is_end_or_error: true,
                    body: Body::json(&Error { name, message }),
                },
                Response::StreamItem { number, item } => {
                    RawPacket::from_stream_item(-(number as i32), item)
                }
            },
        }
    }

    pub fn build(self) -> Vec<u8> {
        self.build_raw().build()
    }
}

#[derive(Clone, PartialEq, Eq)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub enum Body {
    Blob(Vec<u8>),
    String(String),
    // TODO proptest arbritrary json value
    Json(#[cfg_attr(test, proptest(value = "b\"{}\".to_vec()"))] Vec<u8>),
}

impl Body {
    fn parse(body_type: BodyType, data: Vec<u8>) -> Result<Self, PacketParseError> {
        Ok(match body_type {
            BodyType::Binary => Body::Blob(data),
            BodyType::Utf8String => {
                let string = String::from_utf8(data)
                    .map_err(|error| PacketParseError::StringPlayloadEncoding { error })?;
                Body::String(string)
            }
            BodyType::Json => Body::Json(data),
        })
    }

    pub fn json(value: &impl serde::Serialize) -> Self {
        // TODO error
        Self::Json(serde_json::to_vec(value).unwrap())
    }

    pub fn into_json(self) -> Result<Vec<u8>, PacketParseError> {
        match self {
            Body::Blob(_) => Err(PacketParseError::UnexpectedBodyType {
                actual: BodyType::Binary,
                expected: BodyType::Json,
            }),
            Body::String(_) => Err(PacketParseError::UnexpectedBodyType {
                actual: BodyType::Utf8String,
                expected: BodyType::Json,
            }),
            Body::Json(data) => Ok(data),
        }
    }

    fn build(self) -> (BodyType, Vec<u8>) {
        match self {
            Self::Blob(data) => (BodyType::Binary, data),
            Self::String(string) => (BodyType::Utf8String, Vec::from(string)),
            Self::Json(data) => (BodyType::Json, data),
        }
    }
}

impl std::fmt::Debug for Body {
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Blob(data) => fmt.debug_tuple("Blob").field(data).finish(),
            Self::String(string) => fmt.debug_tuple("String").field(string).finish(),
            Self::Json(data) => fmt
                .debug_tuple("Json")
                .field(&String::from_utf8_lossy(data))
                .finish(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
struct RawPacket {
    request_number: i32,
    is_stream: bool,
    is_end_or_error: bool,
    body: Body,
}

impl RawPacket {
    fn header_and_body(self) -> (Header, Vec<u8>) {
        let Self {
            request_number,
            is_stream,
            is_end_or_error,
            body,
        } = self;
        let (body_type, body_data) = body.build();
        let header = Header {
            request_number,
            body_len: body_data.len() as u32,
            body_type,
            flags: HeaderFlags {
                is_stream,
                is_end_or_error,
            },
        };
        (header, body_data)
    }

    fn build(self) -> Vec<u8> {
        let (header, mut body_data) = self.header_and_body();
        let mut data = header.build().to_vec();
        data.append(&mut body_data);
        data
    }

    fn from_stream_item(request_number: i32, stream_item: StreamItem) -> Self {
        Self {
            request_number,
            is_stream: true,
            is_end_or_error: stream_item.is_end(),
            body: stream_item_into_body(stream_item),
        }
    }
}

fn stream_item_into_body(stream_item: StreamItem) -> Body {
    match stream_item {
        StreamItem::Data(body) => body,
        StreamItem::Error(error) => Body::json(&error),
        StreamItem::End => Body::json(&true),
    }
}

fn parse_error_json(json: &[u8]) -> Result<Error, PacketParseError> {
    serde_json::from_slice(json).map_err(|error| PacketParseError::ErrorResponseBody {
        body: String::from_utf8_lossy(&json).into_owned(),
        error,
    })
}

fn parse_stream_item(
    header_flags: &HeaderFlags,
    body: Body,
) -> Result<StreamItem, PacketParseError> {
    let item = if header_flags.is_end_or_error {
        let json = body.into_json()?;
        if json == b"true" {
            StreamItem::End
        } else {
            let error = parse_error_json(&json)?;
            StreamItem::Error(Error {
                name: error.name,
                message: error.message,
            })
        }
    } else {
        StreamItem::Data(body)
    };
    Ok(item)
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::test_utils::*;

    #[proptest]
    fn packet_build_parse(packet: Packet) {
        let (header, body) = packet.clone().build_raw().header_and_body();
        let packet2 = Packet::parse(header, body)?;
        prop_assert_eq!(packet, packet2);
    }
}
