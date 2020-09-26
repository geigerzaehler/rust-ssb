use super::header::BodyType;

use super::error::Error;
pub use super::header::{Header, HeaderFlags, HeaderParseError};
use super::stream_message::StreamMessage;

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
    Stream {
        #[cfg_attr(test, proptest(strategy = "1..(u32::MAX / 2)"))]
        number: u32,
        message: StreamMessage,
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
    Stream {
        #[cfg_attr(test, proptest(strategy = "1..(u32::MAX / 2)"))]
        number: u32,
        message: StreamMessage,
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
                let message = parse_stream_message(&header.flags, body)?;
                Request::Stream { number, message }
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
                let message = parse_stream_message(&header.flags, body)?;
                Response::Stream { number, message }
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
                Request::Stream { number, message } => {
                    RawPacket::from_stream_message(number as i32, message)
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
                Response::Stream { number, message } => {
                    RawPacket::from_stream_message(-(number as i32), message)
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

    fn into_json(self) -> Result<Vec<u8>, PacketParseError> {
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

    /// Deserializes a JSON body into the type `T`.
    ///
    /// Errors when the body does not contain JSON data or the JSON value cannot be decoded as `T`.
    pub fn decode_json<T: serde::de::DeserializeOwned>(&self) -> Result<T, BodyDecodeError> {
        match self {
            Body::Blob(_) => Err(BodyDecodeError::InvalidBodyType {
                actual: BodyType::Binary,
            }),
            Body::String(_) => Err(BodyDecodeError::InvalidBodyType {
                actual: BodyType::Utf8String,
            }),
            Body::Json(data) => Ok(serde_json::from_slice(&data)?),
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

/// Error returned by [Body::decode_json].
#[derive(Debug, thiserror::Error)]
pub enum BodyDecodeError {
    #[error("Invalid body type {actual:?}, expected JSON")]
    InvalidBodyType { actual: BodyType },
    #[error("Failed to decode json")]
    DecodeJson(
        #[from]
        #[source]
        serde_json::Error,
    ),
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

    fn from_stream_message(request_number: i32, stream_message: StreamMessage) -> Self {
        Self {
            request_number,
            is_stream: true,
            is_end_or_error: stream_message.is_end(),
            body: stream_message_into_body(stream_message),
        }
    }
}

fn stream_message_into_body(stream_message: StreamMessage) -> Body {
    match stream_message {
        StreamMessage::Data(body) => body,
        StreamMessage::Error(error) => Body::json(&error),
        StreamMessage::End => Body::json(&true),
    }
}

fn parse_error_json(json: &[u8]) -> Result<Error, PacketParseError> {
    serde_json::from_slice(json).map_err(|error| PacketParseError::ErrorResponseBody {
        body: String::from_utf8_lossy(&json).into_owned(),
        error,
    })
}

fn parse_stream_message(
    header_flags: &HeaderFlags,
    body: Body,
) -> Result<StreamMessage, PacketParseError> {
    let stream_message = if header_flags.is_end_or_error {
        let json = body.into_json()?;
        if json == b"true" {
            StreamMessage::End
        } else {
            let error = parse_error_json(&json)?;
            StreamMessage::Error(Error {
                name: error.name,
                message: error.message,
            })
        }
    } else {
        StreamMessage::Data(body)
    };
    Ok(stream_message)
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
