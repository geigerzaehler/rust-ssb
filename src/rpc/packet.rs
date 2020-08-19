use super::header::BodyType;

pub use super::header::{Header, HeaderParseError};

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub enum Packet {
    Request {
        #[cfg_attr(test, proptest(strategy = "1..(u32::MAX / 2)"))]
        number: u32,
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
    AsyncResponse {
        #[cfg_attr(test, proptest(strategy = "1..(u32::MAX / 2)"))]
        number: u32,
        body: Body,
    },
    AsyncErrorResponse {
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
    #[error("Failed to decode request body")]
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub enum RequestType {
    Async,
    Source,
}

impl RequestType {
    fn as_string(&self) -> String {
        match self {
            Self::Async => "async",
            Self::Source => "source",
        }
        .to_string()
    }

    fn try_from_str(value: &str) -> Result<Self, PacketParseError> {
        match value {
            "async" => Ok(Self::Async),
            "source" => Ok(Self::Source),
            _ => Err(PacketParseError::InvalidRequestType {
                typ: value.to_string(),
            }),
        }
    }

    fn is_stream(&self) -> bool {
        match self {
            Self::Async => false,
            Self::Source => true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
struct RequestBody {
    name: Vec<String>,
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

impl Packet {
    pub fn new(header: Header, body: Vec<u8>) -> Result<Self, PacketParseError> {
        let request_number = header.request_number;
        let is_stream = header.is_stream;
        #[allow(clippy::collapsible_if)]
        let packet = if request_number > 0 {
            if header.is_end_or_error {
                todo!("request end or error")
            } else {
                let RequestBody { name, args, typ } = serde_json::from_slice(&body)
                    .map_err(|error| PacketParseError::RequestBody { body, error })?;
                let typ = RequestType::try_from_str(&typ)?;
                Packet::Request {
                    typ,
                    number: header.request_number as u32,
                    method: name,
                    args,
                }
            }
        } else {
            if header.is_end_or_error {
                let ErrorResponseBody { name, message } = serde_json::from_slice(&body)
                    .map_err(|error| PacketParseError::ErrorResponseBody { body, error })?;
                Packet::AsyncErrorResponse {
                    number: -header.request_number as u32,
                    message,
                    name,
                }
            } else if is_stream {
                todo!("stream response")
            } else {
                let body = Body::new(header.body_type, body);
                Packet::AsyncResponse {
                    number: -header.request_number as u32,
                    body,
                }
            }
        };
        Ok(packet)
    }

    pub fn build_raw(self) -> RawPacket {
        match self {
            Packet::Request {
                number,
                typ,
                method,
                args,
            } => RawPacket::new(
                HeaderOptions {
                    request_number: number as i32,
                    is_stream: typ.is_stream(),
                    is_end_or_error: false,
                },
                Body::json(&RequestBody {
                    name: method,
                    typ: typ.as_string(),
                    args,
                }),
            ),
            Packet::AsyncResponse { number, body } => RawPacket::new(
                HeaderOptions {
                    request_number: -(number as i32),
                    is_stream: false,
                    is_end_or_error: false,
                },
                body,
            ),
            Packet::AsyncErrorResponse {
                number,
                name,
                message,
            } => RawPacket::new(
                HeaderOptions {
                    request_number: -(number as i32),
                    is_stream: false,
                    is_end_or_error: true,
                },
                Body::json(&ErrorResponseBody { message, name }),
            ),
        }
    }

    pub fn build(self) -> Vec<u8> {
        self.build_raw().build()
    }
}

#[derive(Clone, PartialEq, Eq)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub enum Body {
    Blob(#[cfg_attr(test, proptest(filter = "|x| !x.is_empty()"))] Vec<u8>),
    String(#[cfg_attr(test, proptest(filter = "|x| !x.is_empty()"))] String),
    // TODO proptest arbritrary json value
    Json(#[cfg_attr(test, proptest(value = "b\"{}\".to_vec()"))] Vec<u8>),
}

impl Body {
    fn new(body_type: BodyType, data: Vec<u8>) -> Self {
        match body_type {
            BodyType::Binary => Body::Blob(data),
            // TODO handle error
            BodyType::Utf8String => Body::String(String::from_utf8(data).unwrap()),
            BodyType::Json => Body::Json(data),
        }
    }

    fn json(value: &impl serde::Serialize) -> Self {
        Self::Json(serde_json::to_vec(value).unwrap())
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

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
struct HeaderOptions {
    request_number: i32,
    is_stream: bool,
    is_end_or_error: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawPacket {
    pub header: Header,
    pub body: Vec<u8>,
}

impl RawPacket {
    fn new(header_options: HeaderOptions, body: Body) -> Self {
        let HeaderOptions {
            request_number,
            is_stream,
            is_end_or_error,
        } = header_options;
        let (body_type, body_data) = body.build();
        let header = Header {
            request_number,
            body_len: body_data.len() as u32,
            body_type,
            is_stream,
            is_end_or_error,
        };
        Self {
            header,
            body: body_data,
        }
    }

    fn build(self) -> Vec<u8> {
        let Self { header, mut body } = self;
        let mut data = header.build().to_vec();
        data.append(&mut body);
        data
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::test_utils::*;

    #[proptest]
    fn packet_build_new(packet: Packet) {
        let RawPacket { header, body } = packet.clone().build_raw();
        let packet2 = Packet::new(header, body).unwrap();
        prop_assert_eq!(packet, packet2);
    }
}
