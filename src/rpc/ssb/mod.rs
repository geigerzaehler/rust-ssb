use futures::prelude::*;
use std::collections::HashMap;

#[derive(Debug)]
pub struct Client {
    base: crate::rpc::base::Client,
}

impl Client {
    pub fn new<Sink_, Stream_>(sink: Sink_, stream: Stream_) -> Self
    where
        Sink_: Sink<Vec<u8>> + Unpin + 'static,
        Sink_::Error: std::error::Error + Send + Sync + 'static,
        Stream_: TryStream<Ok = Vec<u8>> + Send + Unpin + 'static,
        Stream_::Error: std::error::Error + 'static,
    {
        Client {
            base: crate::rpc::base::Client::new(sink, stream),
        }
    }

    pub fn base(&mut self) -> &mut crate::rpc::base::Client {
        &mut self.base
    }

    pub async fn manifest(&mut self) -> Result<Manifest, Error> {
        let rpc_manifest = self
            .send_async_json::<RpcManifest>(&["manifest"], vec![])
            .await?;
        Ok(Manifest::from(rpc_manifest))
    }

    pub async fn help(&mut self, group: Option<&str>) -> Result<Help, Error> {
        let method = if let Some(group) = group {
            vec![group, "help"]
        } else {
            vec!["help"]
        };
        let help = self.send_async_json::<Help>(&method, vec![]).await?;
        Ok(help)
    }

    async fn send_async_json<T: serde::de::DeserializeOwned>(
        &mut self,
        method: &[&str],
        args: Vec<serde_json::Value>,
    ) -> Result<T, Error> {
        let method = method.iter().map(|s| String::from(*s)).collect();
        let response = self.base.send_async(method, args).await?;

        match response {
            crate::rpc::base::AsyncResponse::Json(data) => {
                let value = serde_json::from_slice::<T>(&data)?;
                Ok(value)
            }
            crate::rpc::base::AsyncResponse::String(_) => {
                Err(Error::InvalidResponseType { type_: "string" })
            }
            crate::rpc::base::AsyncResponse::Blob(_) => {
                Err(Error::InvalidResponseType { type_: "blob" })
            }
            crate::rpc::base::AsyncResponse::Error { name, message } => {
                Err(Error::Rpc { name, message })
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Base(#[from] crate::rpc::base::AsyncRequestError),
    #[error("Failed to decode response")]
    Decode {
        #[from]
        #[source]
        error: serde_json::Error,
    },
    #[error("Invalid response type: {type_}")]
    InvalidResponseType { type_: &'static str },
    #[error("RPC error response ({name}): {message}")]
    Rpc { name: String, message: String },
}

#[derive(Debug)]
pub struct Manifest {
    pub methods: Vec<ManifestMethod>,
    pub groups: HashMap<String, Manifest>,
}

#[derive(Debug)]
pub struct ManifestMethod {
    pub name: String,
    pub type_: String,
}

impl From<RpcManifest> for Manifest {
    fn from(m: RpcManifest) -> Self {
        let mut methods = Vec::new();
        let mut groups = HashMap::new();
        for (name, value) in m.0 {
            match value {
                RpcManifestEntry::Method(type_) => methods.push(ManifestMethod { name, type_ }),
                RpcManifestEntry::Group(group_manifest) => {
                    groups.insert(name, Manifest::from(group_manifest));
                }
            }
        }
        Self { methods, groups }
    }
}

#[derive(serde::Deserialize, Debug)]
struct RpcManifest(HashMap<String, RpcManifestEntry>);

#[derive(serde::Deserialize, Debug)]
#[serde(untagged)]
enum RpcManifestEntry {
    Method(String),
    Group(RpcManifest),
}

#[derive(serde::Deserialize, Debug)]
pub struct Help {
    pub description: String,
    #[serde(rename = "commands")]
    pub methods: HashMap<String, HelpMethod>,
}

#[derive(serde::Deserialize, Debug)]
pub struct HelpMethod {
    pub description: String,
    #[serde(rename = "type")]
    pub type_: String,
    args: HashMap<String, HelpMethodArg>,
}

#[derive(serde::Deserialize, Debug)]
pub struct HelpMethodArg {
    pub description: Option<String>,
    #[serde(rename = "type")]
    pub type_: String,
    #[serde(default)]
    pub optional: bool,
    pub default: Option<serde_json::Value>,
}
