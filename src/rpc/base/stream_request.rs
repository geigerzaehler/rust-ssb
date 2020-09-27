use super::packet::{Body, Request};
use super::stream_message::StreamMessage;

/// Request to start a stream with the server
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct StreamRequest {
    pub name: Vec<String>,
    #[serde(rename = "type")]
    pub type_: StreamRequestType,
    pub args: Vec<serde_json::Value>,
}

impl StreamRequest {
    pub fn into_request(self, id: u32) -> Request {
        StreamMessage::Data(Body::json(&self)).into_request(id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamRequestType {
    /// Only the server sends messages
    Source,
    /// Only the client sends messages
    Sink,
    /// Both the server and the client send messages
    Duplex,
}

impl serde::Serialize for StreamRequestType {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::Source => "source",
            Self::Sink => "sink",
            Self::Duplex => "duplex",
        }
        .serialize(serializer)
    }
}

impl<'de> serde::Deserialize<'de> for StreamRequestType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error;

        let value = String::deserialize(deserializer)?;
        match value.as_ref() {
            "source" => Ok(Self::Source),
            "sink" => Ok(Self::Sink),
            "duplex" => Ok(Self::Duplex),
            value => Err(D::Error::invalid_value(
                serde::de::Unexpected::Str(value),
                &"one of \"source\", \"sink\" or \"duplex\"",
            )),
        }
    }
}
