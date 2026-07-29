mod download;
mod request;

use download::create_directory;
use request::handle_processing;

use ::core::time::Duration;
use std::path::PathBuf;
use clap::{Args, Parser, Subcommand};
use console::style;
use ethers::utils::hex::encode;
use ethers_providers::{Http, Middleware, Provider};
use eyre::Result;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use reqwest::Client;

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
    /// Address as ENS Name or hex (0x1Bca23...)
    address: String,

    /// directory to create nft folder
    #[arg(short, long)]
    path: Option<PathBuf>,

    /// maximum number of parallel downloads
    #[arg(short, long = "max", default_value_t = 5)]
    max_concurrent_downloads: usize,

    /// RPC Url
    #[arg(long, default_value = "https://eth.llamarpc.com")]
    rpc: String,
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
            let provider = Provider::<Http>::try_from(args.rpc)?;
            let account = match args.address {
                arg if arg.split(".").last().unwrap() == "eth" => {
                    let spinner =
                        pending(&multi_pb, "ENS Detected. Resolving address...".to_string());
                    let address = resolve_ens_name(&arg, provider).await?;
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
            };

            let mut path = args
                .path
                .map(PathBuf::from)
                .or_else(|| dirs::picture_dir())
                .unwrap_or_else(|| PathBuf::from("."));
            path.push("nft-folder");

            path = match account.name {
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
            handle_processing(
                &client,
                account.address.as_str(),
                path,
                args.max_concurrent_downloads,
            )
            .await?;

            /*
               :: (4/6) Requesting NFT Data
               :: (5/6) 45 NFTs found. Starting download
            */
            Ok(())
        }
    }
}

async fn resolve_ens_name(ens_name: &str, provider: Provider<Http>) -> Result<String> {
    let address = provider.resolve_name(ens_name).await?;
    Ok(format!("0x{}", encode(address)))
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
