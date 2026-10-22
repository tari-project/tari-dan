//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

/// Fee allowance for every transaction this simulator submits.
///
/// An upper bound, not a payment — the unspent remainder is refunded — so it is set well clear of
/// what these cost rather than tuned to them. A tight value stops the simulator dead the next time
/// the engine's pricing moves.
pub const MAX_FEE: u64 = 50_000;

mod coin;
mod config;
mod sim;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use log::LevelFilter;
use tari_crypto::{ristretto::RistrettoSecretKey, tari_utilities::hex::Hex};
use tari_ootle_common_types::engine_types::published_template::PublishedTemplateAddress;
use tari_template_lib_types::{ResourceAddress, UtxoId};

use crate::{
    coin::Coin,
    config::{Config, WalletConfig},
    sim::TrafficSim,
};

#[derive(Parser)]
#[command(name = "traffic-sim")]
#[command(about = "A CLI tool for simulating traffic between wallets in a swarm")]
struct Cli {
    #[arg(short = 'c', long, default_value = "./config.json", help = "URL of the swarm API")]
    config: PathBuf,
    #[command(subcommand)]
    command: Commands,
}

impl Cli {
    pub fn init() -> Self {
        Cli::parse()
    }
}

#[derive(Subcommand)]
enum Commands {
    Run {
        #[arg(long, default_value_t = 100000000, help = "Minimum transaction value")]
        min_value: u64,

        #[arg(long, default_value_t = 200000000, help = "Maximum transaction value")]
        max_value: u64,

        #[arg(short = 'r', long, help = "Resource address to use for transactions (optional)")]
        resource_address: Option<ResourceAddress>,

        #[arg(
            short = 'c',
            long,
            help = "Path to coin file. If not provided, resource_address must be set"
        )]
        coin_file: Option<PathBuf>,
    },
    Init {
        #[arg(short = 't', long, help = "Stable coin template address")]
        template_address: PublishedTemplateAddress,

        #[arg(short = 'o', long, help = "Coin file output path")]
        coin_file: PathBuf,

        #[arg(
            long,
            value_parser = parse_secret_view_key,
            help = "Optional secret view key (hex) that overrides the wallet-generated view key"
        )]
        view_key: Option<RistrettoSecretKey>,
    },
    Setup {
        #[arg(short = 'c', long, help = "Path to coin file")]
        coin_file: PathBuf,
    },
    ListWallets,
    DecryptUtxos {
        #[arg(long, default_value_t = 0, help = "Minimum transaction value")]
        min_value: u64,

        #[arg(long, default_value_t = 300000000, help = "Maximum transaction value")]
        max_value: u64,
        #[arg(short = 'r', long, help = "Resource address of the resource to decrypt")]
        resource_address: ResourceAddress,

        #[arg(long, help = "Last UTXO ID processed")]
        last_id: Option<String>,

        #[arg(long, help = "Specific UTXO ID to decrypt")]
        specific_id: Option<String>,

        #[arg(long, alias = "csv", help = "Path to output CSV file with results")]
        csv_output: Option<PathBuf>,

        #[arg(
            long,
            value_parser = parse_secret_view_key,
            help = "Secret view key (hex) to decrypt with, if the stablecoin was initialised with an external view key"
        )]
        view_key: Option<RistrettoSecretKey>,
    },
    #[command(alias = "swarm-config")]
    GenerateConfigFromSwarm {
        #[arg(
            long,
            default_value = "http://localhost:8080/json_rpc",
            help = "URL of the swarm API"
        )]
        swarm_url: String,

        #[arg(
            short = 'x',
            long,
            default_value = "http://localhost:5100/json_rpc",
            help = "URL of the exchange wallet API"
        )]
        exchange_wallet_url: String,
        #[arg(short = 'o', long, help = "Output path for the generated config file")]
        output: PathBuf,
    },
}

#[allow(clippy::too_many_lines)]
#[tokio::main]
async fn main() -> Result<()> {
    env_logger::builder().filter_level(LevelFilter::Info).init();
    let cli = Cli::init();

    match cli.command {
        Commands::Run {
            min_value,
            max_value,
            resource_address,
            coin_file,
        } => {
            if min_value >= max_value {
                return Err(anyhow::anyhow!("min-value must be less than max-value"));
            }
            let config = read_json_file::<Config, _>(cli.config)?;
            let coin = coin_file.as_ref().map(read_json_file::<Coin, _>).transpose()?;

            let mut sim = TrafficSim::new(config);
            let resource_address = coin
                .as_ref()
                .map(|c| c.resource_address)
                .or(resource_address)
                .ok_or_else(|| {
                    anyhow::anyhow!("Either resource_address or coin_file must be provided to run the simulation")
                })?;
            sim.run_simulation(resource_address, min_value, max_value).await?;
        },
        Commands::Init {
            template_address,
            coin_file,
            view_key,
        } => {
            let config = read_json_file::<Config, _>(cli.config)?;
            let mut sim = TrafficSim::new(config);
            sim.setup_stablecoin(template_address, &coin_file, view_key).await?;

            println!();
            println!("Stablecoin initialised. Next, fund the simulation wallets:");
            println!("    traffic-sim setup --coin-file {}", coin_file.display());
        },
        Commands::Setup { coin_file } => {
            let config = read_json_file::<Config, _>(cli.config)?;
            let coin = read_json_file::<Coin, _>(&coin_file)?;
            let resource_address = coin.resource_address;

            let mut sim = TrafficSim::new(config);
            sim.connect_to_wallets().await?;
            sim.setup_accounts().await?;
            sim.setup_wallet_funds(coin.component_address, coin.resource_address, coin.admin_badge)
                .await?;

            println!();
            println!("Wallets funded. Start generating traffic with either:");
            println!("    traffic-sim run --coin-file {}", coin_file.display());
            println!("    traffic-sim run --resource-address {resource_address}");
        },
        Commands::ListWallets => {
            let config = read_json_file::<Config, _>(cli.config)?;
            let mut sim = TrafficSim::new(config);
            sim.connect_to_wallets().await?;
            if sim.wallets().is_empty() {
                println!("No wallets found in swarm");
                return Ok(());
            }

            println!("Found {} wallets:", sim.wallets().len());
            for (i, wallet) in sim.wallets().iter().enumerate() {
                println!("{}) {}", i + 1, wallet.client.endpoint());
            }
        },
        Commands::DecryptUtxos {
            min_value,
            max_value,
            resource_address,
            last_id,
            specific_id,
            csv_output,
            view_key,
        } => {
            let config = read_json_file::<Config, _>(cli.config)?;
            let mut sim = TrafficSim::new(config);
            sim.decrypt_utxos(
                min_value,
                max_value,
                resource_address,
                last_id
                    .as_ref()
                    .map(|id| UtxoId::from_hex(id).context("Invalid last_id"))
                    .transpose()?,
                specific_id
                    .as_ref()
                    .map(|id| UtxoId::from_hex(id).context("Invalid specific_id"))
                    .transpose()?,
                csv_output,
                view_key,
            )
            .await?;
        },
        Commands::GenerateConfigFromSwarm {
            swarm_url,
            exchange_wallet_url,
            output,
        } => {
            let wallets = TrafficSim::get_wallets_from_swarm(&swarm_url).await?;
            let config = Config {
                exchange_wallet_url,
                wallets: wallets
                    .iter()
                    .map(|w| WalletConfig {
                        name: w.name.clone(),
                        url: w.client.endpoint().clone(),
                    })
                    .collect(),
            };

            let file = std::fs::File::create(&output).context("Failed to create config file")?;
            serde_json::to_writer_pretty(file, &config).context("Failed to write config file")?;
            println!("Config file written to {}", output.display());
        },
    }

    Ok(())
}

fn read_json_file<T: serde::de::DeserializeOwned, P: AsRef<Path>>(path: P) -> Result<T> {
    let file = std::fs::File::open(path).context("Failed to open JSON file")?;
    let data = serde_json::from_reader(file).context("Failed to parse json file")?;
    Ok(data)
}

fn parse_secret_view_key(s: &str) -> Result<RistrettoSecretKey, String> {
    RistrettoSecretKey::from_hex(s).map_err(|e| format!("Invalid secret view key: {e}"))
}
