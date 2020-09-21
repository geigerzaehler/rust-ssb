use anyhow::Context as _;
use futures::prelude::*;
use structopt::{clap, StructOpt};

pub async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let args = Cli::from_args();
    args.command.run(args.options).await
}

/// Interact with a SSB local server
///
/// Connects to the SSB server using a unix domain socket.
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
    /// Path of Unix socket to connect to the server
    #[structopt(long, default_value(Options::socket_default()))]
    socket: std::path::PathBuf,
}

impl Options {
    async fn client(&self) -> anyhow::Result<crate::rpc::ssb::Client> {
        let stream = async_std::os::unix::net::UnixStream::connect(&self.socket)
            .await
            .context(format!(
                "Failed to connect to {}",
                self.socket.to_string_lossy()
            ))?;
        let (read, write) = stream.split();
        let receive = crate::utils::read_to_stream(read);
        let send = write.into_sink::<Vec<u8>>();

        let client = crate::rpc::ssb::Client::new(send, receive);
        Ok(client)
    }

    // We have to return `&str` instead of `String`. Otherwise we can’t use it the default value
    // for the `socket` option.
    fn socket_default() -> &'static str {
        let home_dir = dirs::home_dir().unwrap();
        let path = home_dir.join(".ssb").join("socket");
        Box::leak(path.to_string_lossy().into_owned().into_boxed_str())
    }
}

#[derive(StructOpt)]
enum Command {
    Call(Call),
    Manifest(Manifest),
    Help(Help),
    PublishPost(PublishPost),
    Invite(Invite),
}

impl Command {
    async fn run(&self, options: Options) -> anyhow::Result<()> {
        match self {
            Self::Call(x) => x.run(options).await,
            Self::Manifest(x) => x.run(options).await,
            Self::Help(x) => x.run(options).await,
            Self::PublishPost(x) => x.run(options).await,
            Self::Invite(x) => x.run(options).await,
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
        use crate::rpc::ssb::Error;
        let mut module = self
            .method
            .split('.')
            .map(std::borrow::ToOwned::to_owned)
            .collect::<Vec<_>>();
        let method = module.pop().unwrap();
        let module = module.get(0).map(AsRef::as_ref);

        let mut client = options.client().await?;
        let module_help = client.help(module).await.map_err(|err| match err {
            Error::Rpc { .. } => anyhow::anyhow!(
                "No help for module `{}` available",
                module.unwrap_or("root")
            ),
            err => anyhow::Error::from(err),
        })?;
        let method_help = module_help
            .methods
            .get(&method)
            .ok_or_else(|| anyhow::anyhow!("Help for method `{}` not available", self.method))?;

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

/// Manage pub invites
#[derive(StructOpt)]
enum Invite {
    Create(InviteCreate),
}

impl Invite {
    async fn run(&self, options: Options) -> anyhow::Result<()> {
        match self {
            Invite::Create(x) => x.run(options).await,
        }
    }
}

/// Create an invite
#[derive(StructOpt)]
struct InviteCreate {
    /// Number of times the invite can be used
    #[structopt(short, default_value = "1")]
    uses: u32,
}

impl InviteCreate {
    async fn run(&self, options: Options) -> anyhow::Result<()> {
        let mut client = options.client().await?;
        let invite = client
            .invite_create(crate::rpc::ssb::InviteCreateParams { uses: self.uses })
            .await?;
        println!("{}", invite);
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
