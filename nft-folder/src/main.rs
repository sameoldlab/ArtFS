mod alchemy;
mod chain;
mod collection;
mod download;
mod helius;
mod opensea;
mod tzkt;

use chain::Chain;
use collection::CollectionState;
use download::{create_directory, save_items};

use ::core::time::Duration;
use alloy::{ens::ProviderEnsExt, providers::ProviderBuilder};
use clap::{Args, Parser, Subcommand};
use console::style;
use dialoguer::{theme::ColorfulTheme, MultiSelect};
use eyre::Result;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use reqwest::Client;
use std::{fmt, path::PathBuf};

#[derive(Parser)]
#[command(version, about, long_about = None)]
#[command(propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Sync a collection: list what's available, let you pick what to keep,
    /// and save the selection to disk. Safe to re-run — only new or
    /// unselected items require your attention, and already-saved files are
    /// never re-downloaded.
    Sync(SyncArgs),
}

#[derive(Args)]
struct SyncArgs {
    /// Account or collection address
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
    chain: Option<String>,

    /// Data source for EVM chains. Options: alchemy, opensea. Defaults to opensea
    #[arg(long)]
    source: Option<Source>,

    /// API key for the selected source.
    /// Reads ALCHEMY_API_KEY, OPENSEA_API_KEY, HELIUS_API_KEY if not given.
    #[arg(long)]
    api_key: Option<String>,

    /// Skip the interactive picker and select every listed item (previously
    /// selected + newly discovered).
    #[arg(long)]
    all: bool,

    /// Skip the interactive picker and keep the current selection as-is
    /// Useful on a re-sync; on a first sync this selects nothing).
    #[arg(long, conflicts_with = "all")]
    no_prompt: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
enum Source {
    Alchemy,
    Tzkt,
    Opensea,
    Helius,
}
impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Source::Alchemy => write!(f, "alchemy"),
            Source::Tzkt => write!(f, "tzkt"),
            Source::Opensea => write!(f, "opensea"),
            Source::Helius => write!(f, "helius"),
        }
    }
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
            let chain = Chain::detect(&args.address, &args.chain.unwrap_or("mainnet".to_string()));
            let source = args.source.unwrap_or_else(|| chain.default_source());

						let account =  if args.address.ends_with(".eth") {
                  let spinner =
                      pending(&multi_pb, "ENS Detected. Resolving address...".to_string());
                  let provider = ProviderBuilder::new().connect_http(args.rpc.parse()?);
                  let address = provider.resolve_name(&args.address).await?.to_string();
                  spinner.finish_with_message(format!("Name Resolved to {address}"));
                  Account {
                      name: Some(args.address.to_string()),
                      address,
                  }
              } else if args.address.ends_with(".tez") || args.address.ends_with(".sol") {
                return Err(eyre::eyre!("Name service not yet supported on this network"));
              }
              else {
                Account {
                    name: None,
                    address: args.address,
                }
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

            let spinner = pending(&multi_pb, format!("Setting up {}", path.to_string_lossy()));
            path = match create_directory(path).await {
                Ok(path) => {
                    spinner.finish();
                    path
                }
                Err(err) => return Err(eyre::eyre!("{} {err}", style("Invalid Path").red())),
            };

            let client = Client::new();


            let mut state =
                CollectionState::load_or_new(&path, &account.address, Some(chain), &source)?;

            let spinner = pending(
                &multi_pb,
                format!("Fetching item list from {source} on {chain}..."),
            );
            let listed: Vec<collection::Item> = match source {
                Source::Alchemy => {
                    let api_key = args
                        .api_key
                        .or_else(|| std::env::var("ALCHEMY_API_KEY").ok())
                        .ok_or_else(|| {
                            eyre::eyre!(
                                "{} Pass --api-key or set ALCHEMY_API_KEY to use this source ",
                                style("Missing API key").red()
                            )
                        })?;
                    alchemy::list_items(&client, &api_key, &account.address).await?
                }
                Source::Opensea => {
                    let api_key = args
                        .api_key
                        .or_else(|| std::env::var("OPENSEA_API_KEY").ok())
                        .ok_or_else(|| {
                            eyre::eyre!(
                                "{} Pass --api-key or set OPENSEA_API_KEY to use this source",
                                style("Missing API key").red()
                            )
                        })?;
                    opensea::list_items(&client, &api_key, &account.address, &chain).await?
                }
                Source::Tzkt => tzkt::list_items(&client, &account.address).await?,
                Source::Helius => {
                    let api_key = args
                        .api_key
                        .or_else(|| std::env::var("HELIUS_API_KEY").ok())
                        .ok_or_else(|| {
                            eyre::eyre!(
                                "{} Pass --api-key or set HELIUS_API_KEY to use this source ",
                                style("Missing API key").red()
                            )
                        })?;

                    helius::list_items(&client, &api_key, &account.address).await?
                }
            };
            spinner.finish_with_message(format!("Found {} items", listed.len()));

            let new_ids = state.merge_listed(listed);
            if !new_ids.is_empty() {
                println!(
                    "{} {} new item(s) since last sync",
                    style("+").green(),
                    new_ids.len()
                );
            }

            if args.all {
                let all_ids: Vec<String> = state.items.keys().cloned().collect();
                state.set_selected(&all_ids);
            } else if !args.no_prompt {
                // Sort for a stable, readable prompt order.
                let mut entries: Vec<(&String, &collection::ItemState)> =
                    state.items.iter().collect();
                entries.sort_by(|a, b| a.1.item.name.cmp(&b.1.item.name));

                let labels: Vec<String> = entries
                    .iter()
                    .map(|(_, s)| {
                        let size = s
                            .item
                            .size_bytes
                            .map(|b| human_size(b))
                            .unwrap_or_else(|| "?".to_string());
                        let flag = if s.downloaded {
                            " [saved]"
                        } else if new_ids.contains(&s.item.id) {
                            " [new]"
                        } else {
                            ""
                        };
                        format!("{} ({size}){flag}", s.item.name)
                    })
                    .collect();

                // Default-check anything already selected, plus anything new,
                // but leave previously-deselected old items alone
                let defaults: Vec<bool> = entries
                    .iter()
                    .map(|(id, s)| s.selected || new_ids.contains(*id))
                    .collect();

                if !entries.is_empty() {
                    let selection = MultiSelect::with_theme(&ColorfulTheme::default())
                        .with_prompt("Select items to save (space to toggle, enter to confirm)")
                        .items(&labels)
                        .defaults(&defaults)
                        .interact()?;

                    let selected_ids: Vec<String> = selection
                        .into_iter()
                        .map(|i| entries[i].0.clone())
                        .collect();
                    state.set_selected(&selected_ids);
                }
            }
            // args.no_prompt: keep selection as currently persisted, untouched.

            let pending_items: Vec<(String, collection::Item)> = state
                .pending()
                .into_iter()
                .map(|s| (s.item.id.clone(), s.item.clone()))
                .collect();

            if pending_items.is_empty() {
                println!("Nothing new to save.");
            } else {
                let saved = save_items(
                    &client,
                    pending_items,
                    path.clone(),
                    args.max_concurrent_downloads,
                )
                .await?;
                for (id, file_name) in saved {
                    state.mark_downloaded(&id, file_name);
                }
            }

            state.touch_synced();
            state.save(&path)?;

            Ok(())
        }
    }
}

fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes}{}", UNITS[unit])
    } else {
        format!("{size:.1}{}", UNITS[unit])
    }
}

/// Wraps a generic action with a spinner then return it's result
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
