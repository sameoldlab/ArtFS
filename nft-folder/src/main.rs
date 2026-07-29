mod alchemy;
mod chain;
mod download;
mod request;
mod tzkt;

use chain::Chain;
use download::create_directory;
use request::{handle_processing, save_tokens};

use ::core::time::Duration;
use alloy::{ens::ProviderEnsExt, providers::ProviderBuilder};
use clap::{Args, Parser, Subcommand};
use console::style;
use eyre::Result;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use reqwest::Client;
use std::path::PathBuf;

#[derive(Parser)]
#[command(version, about, long_about = None)]
#[command(propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Create a folder for the provided address
    Sync(SyncArgs),
}

#[derive(Args)]
struct SyncArgs {
    /// Address as ENS/Tezos name, hex (0x1Bca23...), or Tezos address (tz1.../KT1...)
    address: String,

    /// directory to create nft folder
    #[arg(short, long)]
    path: Option<PathBuf>,

    /// maximum number of parallel downloads
    #[arg(short, long = "max", default_value_t = 5)]
    max_concurrent_downloads: usize,

    /// RPC Url
    #[arg(long, default_value = "https://ethereum-rpc.publicnode.com")]
    rpc: String,

    /// Which network to pull from. Auto-detected from the address format if not given
    #[arg(long)]
    chain: Option<Chain>,

    /// Data source for EVM chains. "zora" uses Zora's public GraphQL API
    /// (no key needed, Ethereum mainnet only). "alchemy" uses the Alchemy
    /// NFT API and requires --api-key or ALCHEMY_API_KEY. Ignored for Tezos.
    #[arg(long, default_value = "zora")]
    source: EvmSource,

    /// API key for the selected source (currently only needed for Alchemy).
    /// Falls back to the ALCHEMY_API_KEY environment variable if not given.
    #[arg(long)]
    api_key: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
enum EvmSource {
    Zora,
    Alchemy,
}

struct Account {
    name: Option<String>,
    address: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Sync(args) => {
            let multi_pb = MultiProgress::new();
            let chain = args.chain.unwrap_or_else(|| Chain::detect(&args.address));

            let account = match chain {
                Chain::Tezos => {
                    if !(args.address.starts_with("tz1")
                        || args.address.starts_with("tz2")
                        || args.address.starts_with("tz3")
                        || args.address.starts_with("KT1"))
                    {
                      // TODO: Tezos names?
                        return Err(eyre::eyre!(
                            "{} Tezos addresses must start with tz1, tz2, tz3, or KT1",
                            style("Invalid address").red()
                        ));
                    }
                    Account {
                        name: None,
                        address: args.address.clone(),
                    }
                }
                Chain::Ethereum => match args.address.as_str() {
                    arg if arg.split(".").last().unwrap() == "eth" => {
                        let spinner =
                            pending(&multi_pb, "ENS Detected. Resolving address...".to_string());
                        let address = resolve_ens_name(arg, &args.rpc.clone()).await?;
                        spinner.finish_with_message(format!("Name Resolved to {address}"));
                        Account {
                            name: Some(arg.to_string()),
                            address,
                        }
                    }
                    arg if arg.starts_with("0x") => Account {
                        name: None,
                        address: arg.to_string(),
                    },
                    _ => {
                        return Err(eyre::eyre!(
                            "{} Supported formats are 0xabc12... or name.eth",
                            style("Invalid address").red()
                        ))
                    }
                },
            };

            let mut path = args
                .path
                .map(PathBuf::from)
                .or_else(|| dirs::picture_dir())
                .unwrap_or_else(|| PathBuf::from("."));
            path.push("nft-folder");

            path = match &account.name {
                Some(name) => path.join(name),
                None => path.join(&account.address),
            };

            let spinner = pending(
                &multi_pb,
                format!("Saving files to {}", path.to_string_lossy()),
            );
            path = match create_directory(path).await {
                Ok(path) => {
                    spinner.finish();
                    path
                }
                Err(err) => return Err(eyre::eyre!("{} {err}", style("Invalid Path").red())),
            };

            let client = Client::new();

            match chain {
                Chain::Ethereum => match args.source {
                    EvmSource::Zora => {
                        handle_processing(
                            &client,
                            account.address.as_str(),
                            path,
                            args.max_concurrent_downloads,
                        )
                        .await?;
                    }
                    EvmSource::Alchemy => {
                        let api_key = args
                            .api_key
                            .or_else(|| std::env::var("ALCHEMY_API_KEY").ok())
                            .ok_or_else(|| {
                                eyre::eyre!(
                                    "{} Pass --api-key or set ALCHEMY_API_KEY to use --source alchemy",
                                    style("Missing API key").red()
                                )
                            })?;

                        let spinner =
                            pending(&multi_pb, "Fetching NFTs from Alchemy...".to_string());
                        let tokens =
                            alchemy::fetch_all(&client, &api_key, &account.address).await?;
                        spinner.finish_with_message(format!("Found {} NFTs", tokens.len()));

                        save_tokens(&client, tokens, path, args.max_concurrent_downloads).await?;
                    }
                },
                Chain::Tezos => {
                    let spinner = pending(&multi_pb, "Fetching NFTs from TzKT...".to_string());
                    let tokens = tzkt::fetch_all(&client, &account.address).await?;
                    spinner.finish_with_message(format!("Found {} NFTs", tokens.len()));

                    save_tokens(&client, tokens, path, args.max_concurrent_downloads).await?;
                }
            }

            Ok(())
        }
    }
}

async fn resolve_ens_name(ens_name: &str, rpc_url: &str) -> Result<String> {
    let provider = ProviderBuilder::new().connect_http(rpc_url.parse()?);
    let address = provider.resolve_name(ens_name).await?;
    Ok(address.to_string())
}

/// Wrapsa generic action with a spinner then return it's result
fn pending(multi_pb: &MultiProgress, msg: String) -> ProgressBar {
    // https://github.com/sindresorhus/cli-spinners/blob/main/spinners.json
    let style = ProgressStyle::default_spinner()
        .template("{spinner:.green} {prefix:.bold.blue} {msg}")
        .unwrap()
        .tick_strings(&["⣼", "⣹", "⢻", "⠿", "⡟", "⣏", "⣧", "⣶", "✔"]);
    let spinner = multi_pb.add(ProgressBar::new_spinner().with_style(style));
    spinner.set_prefix("INFO");
    spinner.set_message(msg);
    spinner.enable_steady_tick(Duration::from_millis(100));

    spinner
}
