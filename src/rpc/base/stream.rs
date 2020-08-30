use super::packet::{Body, Response};
use futures::prelude::*;

#[derive(Debug)]
pub enum RpcStreamItem {
    Data(Body),
    // TODO error
}

pub(super) async fn forward_rpc_stream<Sink_>(
    request_number: u32,
    mut stream: impl Stream<Item = RpcStreamItem> + Unpin,
    mut response_sink: Sink_,
) -> Result<(), Sink_::Error>
where
    Sink_: Sink<Response> + Unpin,
{
    while let Some(item) = stream.next().await {
        let response = match item {
            RpcStreamItem::Data(body) => Response::StreamItem {
                number: request_number,
                body,
            },
        };

        response_sink.send(response).await?;
    }
    response_sink
        .send(Response::StreamEnd {
            number: request_number,
        })
        .await
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub(super) struct StreamRequest {
    pub(super) name: Vec<String>,
    #[serde(rename = "type")]
    pub(super) type_: RequestType,
    // TODO generate json values
    #[cfg_attr(test, proptest(value = "vec![]"))]
    pub(super) args: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[allow(dead_code)]
pub(super) enum RequestType {
    Source,
    Sink,
    Duplex,
}

impl serde::Serialize for RequestType {
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

impl<'de> serde::Deserialize<'de> for RequestType {
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
