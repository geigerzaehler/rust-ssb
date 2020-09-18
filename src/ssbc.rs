use crate::{crypto, handshake, SCUTTLEBUTT_NETWORK_IDENTIFIER};
use anyhow::Context as _;
use structopt::{clap, StructOpt};

pub async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let args = Cli::from_args();
    args.command.run(args.options).await
}

/// Interact with a SSB server
#[derive(StructOpt)]
#[structopt(
    name = "ssbc",
    max_term_width = 100,
    setting(clap::AppSettings::UnifiedHelpMessage)
)]
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

    /// `hostname:port` pair to connect to the server.
    #[structopt(long, default_value = "localhost:8008")]
    server: String,

    /// Base64 encoded public key of the server
    #[structopt(long, parse(try_from_str = Options::parse_server_id))]
    server_id: Option<crypto::sign::PublicKey>,
}

impl Options {
    async fn client(&self) -> anyhow::Result<crate::rpc::ssb::Client> {
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
            &SCUTTLEBUTT_NETWORK_IDENTIFIER,
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
        let client = crate::rpc::ssb::Client::new(encrypt, decrypt);
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
    Manifest(Manifest),
    Help(Help),
    PublishPost(PublishPost),
}

impl Command {
    async fn run(&self, options: Options) -> anyhow::Result<()> {
        match self {
            Self::Call(x) => x.run(options).await,
            Self::Manifest(x) => x.run(options).await,
            Self::Help(x) => x.run(options).await,
            Self::PublishPost(x) => x.run(options).await,
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
        let response = client.base().send_async(method, vec![]).await?;
        let response = match response {
            crate::rpc::base::AsyncResponse::Json(data) => {
                let value = serde_json::from_slice::<serde_json::Value>(&data)
                    .context("Failed to decode response")?;
                serde_json::to_string_pretty(&value).unwrap()
            }
            crate::rpc::base::AsyncResponse::String(string) => string,
            crate::rpc::base::AsyncResponse::Blob(_data) => {
                "Refusing to print binary data".to_string()
            }
            crate::rpc::base::AsyncResponse::Error { name, message } => {
                anyhow::bail!("RPC error \"{}\": {}", name, message)
            }
        };
        println!("{}", response);
        Ok(())
    }
}

#[derive(StructOpt)]
/// Prints RPC methods the server supports
struct Manifest {}

impl Manifest {
    async fn run(&self, options: Options) -> anyhow::Result<()> {
        let mut client = options.client().await?;

        let manifest = client.manifest().await?;

        let mut table = new_table();
        table.set_titles(prettytable::row![b => "METHOD", "TYPE", "DESCRIPTION"]);

        let help = client.help(None).await?;

        for (name, command) in help.methods {
            table.add_row(prettytable::row![i -> name, command.type_, command.description]);
        }

        for (group, group_manifest) in manifest.modules {
            if group_manifest
                .methods
                .iter()
                .any(|method| method.name == "help")
            {
                let help = client.help(Some(&*group)).await?;
                for (name, command) in help.methods {
                    table.add_row(prettytable::row![i -> format!("{}.{}", group, name), command.type_, command.description]);
                }
            } else {
                for method in group_manifest.methods {
                    table.add_row(
                        prettytable::row![i -> format!("{}.{}", group, method.name), method.type_],
                    );
                }
            }
        }

        table.printstd();

        Ok(())
    }
}

/// Print help for an RPC method
#[derive(StructOpt)]
struct Help {
    /// Method path delimited with a dot (.)
    method: String,
}

impl Help {
    async fn run(&self, options: Options) -> anyhow::Result<()> {
        let mut module = self
            .method
            .split('.')
            .map(std::borrow::ToOwned::to_owned)
            .collect::<Vec<_>>();
        let method = module.pop().unwrap();

        let mut client = options.client().await?;
        let module_help = client.help(module.get(1).map(AsRef::as_ref)).await?;
        let method_help = module_help.methods.get(&method).ok_or(anyhow::anyhow!(
            "Help for method `{}` not available",
            self.method
        ))?;

        let mut table = new_table();
        table.add_row(prettytable::row!["NAME", method]);
        table.add_row(prettytable::row!["TYPE", method_help.type_]);
        table.add_row(prettytable::row!["DESCRIPTION", method_help.description]);
        table.printstd();
        Ok(())
    }
}

/// Publish a post
#[derive(StructOpt)]
struct PublishPost {
    /// Text content of the post
    text: String,
}

impl PublishPost {
    async fn run(&self, options: Options) -> anyhow::Result<()> {
        let mut client = options.client().await?;
        let message = client
            .publish(crate::rpc::ssb::MessageContent {
                type_: "post".to_string(),
                text: self.text.clone(),
            })
            .await?;
        println!("{}", serde_json::to_string_pretty(&message).unwrap());
        Ok(())
    }
}

fn new_table() -> prettytable::Table {
    let mut table = prettytable::Table::new();
    let format = prettytable::format::FormatBuilder::new()
        .column_separator(' ')
        .padding(0, 2)
        .build();
    table.set_format(format);
    table
}
