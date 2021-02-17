//! Provides [Client] for the SSB RPC protocol.
use futures::prelude::*;
use std::collections::HashMap;

#[derive(Debug)]
pub struct Client {
    endpoint: muxrpc::Endpoint,
}

impl Client {
    /// Create a new client from a duplex raw byte connection with a server.
    ///
    /// See [muxrpc::Client] for details.
    pub fn new<Sink_, Stream_>(send: Sink_, receive: Stream_) -> Self
    where
        Sink_: Sink<Vec<u8>> + Send + Unpin + 'static,
        Sink_::Error: std::error::Error + Send + Sync + 'static,
        Stream_: TryStream<Ok = Vec<u8>> + Send + Unpin + 'static,
        Stream_::Error: std::error::Error + Send + Sync + 'static,
    {
        Client {
            endpoint: muxrpc::Endpoint::new_client(send, receive),
        }
    }

    /// Get the underlying application agnostic client.
    pub fn base(&mut self) -> &mut muxrpc::Client {
        self.endpoint.client()
    }

    /// Get all registered RPC methods .
    pub async fn manifest(&mut self) -> Result<Manifest, Error> {
        let rpc_manifest = self
            .send_async_json::<RpcManifest>(&["manifest"], vec![])
            .await?;
        Ok(Manifest::from(rpc_manifest))
    }

    /// Get description and signature information of available RPC methods for
    /// the given module.
    ///
    /// If `module` is given calls the method `[module, "help"]`. Otherwise just calls `["help"]`.
    ///
    /// Not all methods may be included in the result. Use [Client::manifest] to
    /// get a comprehensive list.
    pub async fn help(&mut self, module: Option<&str>) -> Result<Help, Error> {
        let method = if let Some(module) = module {
            vec![module, "help"]
        } else {
            vec!["help"]
        };
        let help = self.send_async_json::<Help>(&method, vec![]).await?;
        Ok(help)
    }

    pub async fn publish(&mut self, content: MessageContent) -> Result<serde_json::Value, Error> {
        self.send_async_json(&["publish"], vec![serde_json::to_value(content).unwrap()])
            .await
    }

    /// Create an invitation
    pub async fn invite_create(&mut self, params: InviteCreateParams) -> Result<String, Error> {
        let response = self
            .endpoint
            .client()
            .send_async(
                vec!["invite".to_string(), "create".to_string()],
                vec![serde_json::to_value(params).unwrap()],
            )
            .await?;

        match response {
            muxrpc::AsyncResponse::Json(_) => Err(Error::InvalidResponseType { type_: "json" }),
            muxrpc::AsyncResponse::String(content) => Ok(content),
            muxrpc::AsyncResponse::Blob(_) => Err(Error::InvalidResponseType { type_: "blob" }),
            muxrpc::AsyncResponse::Error(error) => Err(Error::Rpc {
                name: error.name,
                message: error.message,
            }),
        }
    }

    /// Send an `async` type request and expect a response with `T` serialized as.
    async fn send_async_json<T: serde::de::DeserializeOwned>(
        &mut self,
        method: &[&str],
        args: Vec<serde_json::Value>,
    ) -> Result<T, Error> {
        let method = method.iter().map(|s| String::from(*s)).collect();
        let response = self.endpoint.client().send_async(method, args).await?;

        match response {
            muxrpc::AsyncResponse::Json(data) => {
                let value = serde_json::from_slice::<T>(&data)?;
                Ok(value)
            }
            muxrpc::AsyncResponse::String(_) => Err(Error::InvalidResponseType { type_: "string" }),
            muxrpc::AsyncResponse::Blob(_) => Err(Error::InvalidResponseType { type_: "blob" }),
            muxrpc::AsyncResponse::Error(error) => Err(Error::Rpc {
                name: error.name,
                message: error.message,
            }),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Base(#[from] muxrpc::AsyncRequestError),
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
    pub modules: HashMap<String, Manifest>,
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
                RpcManifestEntry::Module(group_manifest) => {
                    groups.insert(name, Manifest::from(group_manifest));
                }
            }
        }
        Self {
            methods,
            modules: groups,
        }
    }
}

#[derive(serde::Deserialize, Debug)]
/// Transport object for [Client::manifest]. Is converted to [Manifest]
struct RpcManifest(HashMap<String, RpcManifestEntry>);

#[derive(serde::Deserialize, Debug)]
#[serde(untagged)]
enum RpcManifestEntry {
    Method(String),
    Module(RpcManifest),
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
    /// The type of the method. Usually one of sync, async, source, sink, or duplex.
    // TODO use enum
    pub type_: String,
    pub args: HashMap<String, HelpMethodArg>,
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

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct MessageContent {
    #[serde(rename = "type")]
    pub type_: String,
    pub text: String,
}

/// Parameters for [Client::invite_create].
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct InviteCreateParams {
    /// Number of times this invite can be used
    pub uses: u32,
}
