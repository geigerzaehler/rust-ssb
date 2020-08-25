use super::header::BodyType;

pub use super::header::{Header, HeaderFlags, HeaderParseError};

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
        // TODO superfluous. Should always by RequestType::Async
        typ: RequestType,
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
        #[cfg_attr(test, proptest(strategy = "proptest::bool::ANY"))]
        is_end: bool,
        body: Body,
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
        body: Body,
    },
    StreamEnd {
        #[cfg_attr(test, proptest(strategy = "1..(u32::MAX / 2)"))]
        number: u32,
    },
    StreamError {
        #[cfg_attr(test, proptest(strategy = "1..(u32::MAX / 2)"))]
        number: u32,
        name: String,
        message: String,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum PacketParseError {
    #[error("Invalid request type {typ}")]
    InvalidRequestType { typ: String },
    #[error("Failed to decode JSON request body")]
    RequestBody {
        body: Vec<u8>,
        #[source]
        error: serde_json::Error,
    },
    #[error("Failed to decode error response body")]
    ErrorResponseBody {
        body: Vec<u8>,
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

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub enum RequestType {
    Async,
    Source,
    Sink,
    Duplex,
}

impl RequestType {
    fn as_string(&self) -> String {
        match self {
            Self::Async => "async",
            Self::Source => "source",
            Self::Sink => "sink",
            Self::Duplex => "duplex",
        }
        .to_string()
    }

    fn try_from_str(value: &str) -> Result<Self, PacketParseError> {
        match value {
            "async" => Ok(Self::Async),
            "source" => Ok(Self::Source),
            "sink" => Ok(Self::Sink),
            "duplex" => Ok(Self::Duplex),
            _ => Err(PacketParseError::InvalidRequestType {
                typ: value.to_string(),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
struct RequestBody {
    name: Vec<String>,
    // TODO generate json values
    #[cfg_attr(test, proptest(value = "vec![]"))]
    args: Vec<serde_json::Value>,
    #[serde(rename = "type")]
    typ: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
struct ErrorResponseBody {
    message: String,
    name: String,
}

impl ErrorResponseBody {
    fn parse(body: Body) -> Result<Self, PacketParseError> {
        let json = body.into_json()?;
        Self::parse_json(&json)
    }

    fn parse_json(json: &[u8]) -> Result<Self, PacketParseError> {
        serde_json::from_slice(&json).map_err(|error| PacketParseError::ErrorResponseBody {
            body: json.to_vec(),
            error,
        })
    }
}

impl Packet {
    pub fn parse(header: Header, body: Vec<u8>) -> Result<Self, PacketParseError> {
        let request_number = header.request_number;
        #[allow(clippy::collapsible_if)]
        let packet = if request_number > 0 {
            if header.flags.is_stream {
                Packet::Request(Request::Stream {
                    number: request_number as u32,
                    is_end: header.flags.is_end_or_error,
                    body: Body::parse(header.body_type, body)?,
                })
            } else {
                if header.flags.is_end_or_error {
                    tracing::error!(?header, ?body);
                    todo!("request end or error")
                }
                let RequestBody { name, args, typ } = serde_json::from_slice(&body)
                    .map_err(|error| PacketParseError::RequestBody { body, error })?;
                let typ = RequestType::try_from_str(&typ)?;
                Packet::Request(Request::Async {
                    typ,
                    number: header.request_number as u32,
                    method: name,
                    args,
                })
            }
        } else {
            let number = -header.request_number as u32;
            let response = if header.flags.is_stream {
                let body = Body::parse(header.body_type, body)?;
                if header.flags.is_end_or_error {
                    let json = body.into_json()?;
                    if json == b"true" {
                        Response::StreamEnd { number }
                    } else {
                        let error = ErrorResponseBody::parse_json(&json)?;
                        Response::StreamError {
                            number,
                            name: error.name,
                            message: error.message,
                        }
                    }
                } else {
                    Response::StreamItem { number, body }
                }
            } else {
                if header.flags.is_end_or_error {
                    let body = Body::parse(header.body_type, body)?;
                    let error = ErrorResponseBody::parse(body)?;
                    Response::AsyncErr {
                        number,
                        name: error.name,
                        message: error.message,
                    }
                } else {
                    let body = Body::parse(header.body_type, body)?;
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
                    typ,
                    method,
                    args,
                } => RawPacket {
                    request_number: number as i32,
                    is_stream: false,
                    is_end_or_error: false,
                    body: Body::json(&RequestBody {
                        name: method,
                        typ: typ.as_string(),
                        args,
                    }),
                },
                Request::Stream {
                    number,
                    is_end,
                    body,
                } => RawPacket {
                    request_number: number as i32,
                    is_stream: true,
                    is_end_or_error: is_end,
                    body,
                },
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
                    body: Body::json(&ErrorResponseBody { name, message }),
                },

                Response::StreamItem { number, body } => RawPacket {
                    request_number: -(number as i32),
                    is_stream: true,
                    is_end_or_error: false,
                    body,
                },
                Response::StreamEnd { number } => RawPacket {
                    request_number: -(number as i32),
                    is_stream: true,
                    is_end_or_error: true,
                    body: Body::json(&true),
                },
                Response::StreamError {
                    number,
                    name,
                    message,
                } => RawPacket {
                    request_number: -(number as i32),
                    is_stream: true,
                    is_end_or_error: true,
                    body: Body::json(&ErrorResponseBody { name, message }),
                },
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

    fn json(value: &impl serde::Serialize) -> Self {
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
