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
        let server_identity_pk = client_identity_pk;

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
}

#[derive(StructOpt)]
enum Command {
    GossipPeers(GossipPeers),
}

impl Command {
    async fn run(&self, options: Options) -> anyhow::Result<()> {
        match self {
            Self::GossipPeers(cmd) => cmd.run(options).await,
        }
    }
}

#[derive(StructOpt)]
/// List peers of the gossip protocol the server is connected to
struct GossipPeers {}

impl GossipPeers {
    async fn run(&self, options: Options) -> anyhow::Result<()> {
        let mut client = options.client().await?;
        let response = client
            .send_async(vec!["gossip".to_string(), "peers".to_string()], vec![])
            .await?;

        let response = match response {
            crate::rpc::AsyncResponse::Json(data) => {
                let value = serde_json::from_slice::<serde_json::Value>(&data)
                    .context("Failed to decode response")?;
                serde_json::to_string_pretty(&value).unwrap()
            }
            crate::rpc::AsyncResponse::String(string) => string,
            other => format!("{:#?}", other),
        };
        println!("{}", response);
        Ok(())
    }
}
