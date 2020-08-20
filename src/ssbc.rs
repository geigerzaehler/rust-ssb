use crate::{crypto, handshake, SCUTTLEBUT_NETWORK_IDENTIFIER};
use anyhow::Context as _;
use structopt::StructOpt;

pub async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let args = Cli::from_args();
    args.command.run(args.options).await
}

/// Interact with a SSB server
#[derive(StructOpt)]
#[structopt(name = "ssbc", max_term_width = 100)]
struct Cli {
    #[structopt(subcommand)]
    command: Command,

    #[structopt(flatten)]
    options: Options,
}

#[derive(StructOpt)]
struct Options {
    #[structopt(long)]
    /// File to load the secret key from. Defaults to ~/.ssb/secret.
    secret_file: Option<std::path::PathBuf>,

    /// Generate an emphemeral identity and don’t load an existing secret key.
    #[structopt(long)]
    anonymous: bool,

    #[structopt(long, default_value = "localhost:8008")]
    server: String,

    /// Base64 encoded public key of the server
    #[structopt(long, parse(try_from_str = Options::parse_server_id))]
    server_id: Option<crypto::sign::PublicKey>,
}

impl Options {
    async fn client(&self) -> anyhow::Result<crate::rpc::Client> {
        let client_identity_sk = if self.anonymous {
            crypto::sign::gen_keypair().1
        } else {
            if let Some(ref secret_file) = self.secret_file {
                crate::secret_file::load(secret_file)
            } else {
                crate::secret_file::load_default()
            }
            .context("Failed to load secret key")?
        };

        let client_identity_pk = client_identity_sk.public_key();
        let server_identity_pk = self.server_id.unwrap_or(client_identity_pk);

        let client = handshake::Client::new(
            &SCUTTLEBUT_NETWORK_IDENTIFIER,
            &server_identity_pk,
            &client_identity_pk,
            &client_identity_sk,
        );

        let stream = async_std::net::TcpStream::connect(&self.server)
            .await
            .context(format!("Failed to connect to {}", &self.server))?;

        let (encrypt, decrypt) = client
            .connect(stream)
            .await
            .context("Failed to establish encrypted connection with server")?;
        let client = crate::rpc::Client::new(encrypt, decrypt);
        Ok(client)
    }

    fn parse_server_id(value: &str) -> anyhow::Result<crypto::sign::PublicKey> {
        let bytes = base64::decode(value)?;
        if bytes.len() != crypto::sign::PUBLICKEYBYTES {
            anyhow::bail!("invalid size public key size");
        }
        Ok(crypto::sign::PublicKey::from_slice(&bytes).unwrap())
    }
}

#[derive(StructOpt)]
enum Command {
    Call(Call),
}

impl Command {
    async fn run(&self, options: Options) -> anyhow::Result<()> {
        match self {
            Self::Call(cmd) => cmd.run(options).await,
        }
    }
}
#[derive(StructOpt)]
/// Call an RPC method without arguments and print the response
struct Call {
    /// Method path delimited with a dot (.)
    method: String,
}

impl Call {
    async fn run(&self, options: Options) -> anyhow::Result<()> {
        let method = self
            .method
            .split('.')
            .map(std::borrow::ToOwned::to_owned)
            .collect();

        let mut client = options.client().await?;
        let response = client.send_async(method, vec![]).await?;
        let response = match response {
            crate::rpc::AsyncResponse::Json(data) => {
                let value = serde_json::from_slice::<serde_json::Value>(&data)
                    .context("Failed to decode response")?;
                serde_json::to_string_pretty(&value).unwrap()
            }
            crate::rpc::AsyncResponse::String(string) => string,
            crate::rpc::AsyncResponse::Blob(_data) => "Refusing to print binary data".to_string(),
            crate::rpc::AsyncResponse::Error { name, message } => {
                anyhow::bail!("RPC error \"{}\": {}", name, message)
            }
        };
        println!("{}", response);
        Ok(())
    }
}
