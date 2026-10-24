//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

mod cli;
mod transaction_writer;

use std::{
    collections::HashMap,
    fs,
    io,
    io::{BufRead, Seek, SeekFrom, Write, stdout},
};

use anyhow::anyhow;
use bounded_vec::BoundedVec;
use cli::Cli;
use tari_crypto::{keys::SecretKey, ristretto::RistrettoSecretKey, tari_utilities::hex::Hex};
use tari_engine_types::substate::SubstateId;
use tari_indexer_client::{
    rest_api_client::IndexerRestApiClient,
    types::{GetSubstatesRequest, MAX_SUBSTATES_PER_FETCH},
};
use tari_ootle_common_types::{Epoch, InputDeclaration};
use tari_ootle_transaction::{Blob, Network};
use tari_template_lib_types::{TemplateAddress, VaultId};
use tari_transaction_manifest::ManifestValue;
use transaction_generator::{
    BoxedTransactionBuilder,
    read_number_of_transactions,
    read_transactions,
    transaction_builders::{
        accounts::{account_address, account_keypair},
        free_coins,
        manifest,
        transfers,
    },
};
use url::Url;

use crate::{
    cli::{Flood, SubCommand, WriteArgs},
    transaction_writer::write_transactions,
};

fn main() -> anyhow::Result<()> {
    let cli = Cli::init();
    match cli.sub_command {
        SubCommand::Write(args) => {
            if !args.overwrite && args.output_file.exists() {
                anyhow::bail!("Output file {} already exists", args.output_file.display());
            }

            let timer = std::time::Instant::now();
            println!("Generating and writing {} transactions", args.num_transactions,);

            let mut file = std::fs::File::create(&args.output_file)?;

            let max_epoch = resolve_max_epoch(&args)?;
            println!("Transactions are valid until epoch {max_epoch}");

            let builder = get_transaction_builder(&args, max_epoch)?;
            write_transactions(
                args.num_transactions,
                builder,
                &|_| {
                    print!(".");
                    stdout().flush().unwrap()
                },
                &mut file,
            )?;
            println!();
            let size = file.metadata()?.len() / 1024 / 1024;
            println!(
                "Wrote {} transactions to {} ({} MiB) in {:.2?}",
                args.num_transactions,
                args.output_file.display(),
                size,
                timer.elapsed()
            );
        },
        SubCommand::Read(args) => {
            let mut file = fs::File::open(args.input_file)?;

            let num_transactions = read_number_of_transactions(&mut file)?;
            println!("Number of transactions: {}", num_transactions);
            file.seek(SeekFrom::Start(0))?;
            let receiver = read_transactions(file, 0)?;

            while let Ok(transaction) = receiver.recv() {
                println!("Read transaction: {}", transaction.calculate_id());
            }
        },
    }

    Ok(())
}

/// The epoch the generated transactions expire in: either the operator's explicit choice, or the
/// indexer's current epoch plus the requested window.
///
/// `max_epoch` is signed over, so it is fixed at generation time and cannot be refreshed when the
/// file is submitted. A file whose window has passed by then is rejected transaction by transaction,
/// which is why this reads the live epoch rather than making the operator supply a number.
fn resolve_max_epoch(args: &WriteArgs) -> anyhow::Result<Epoch> {
    if let Some(max_epoch) = args.max_epoch {
        return Ok(Epoch(max_epoch));
    }

    let client = IndexerRestApiClient::connect(args.indexer_url.clone())?;
    let stats = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(client.get_epoch_manager_stats())
        .map_err(|e| {
            anyhow!(
                "Could not read the current epoch from indexer {}: {e}. Pass --max-epoch to choose the validity \
                 window without an indexer.",
                args.indexer_url
            )
        })?;

    Ok(Epoch(stats.current_epoch.as_u64().saturating_add(args.validity_epochs)))
}

fn get_transaction_builder(args: &WriteArgs, max_epoch: Epoch) -> anyhow::Result<BoxedTransactionBuilder> {
    let network = args.network.unwrap_or(Network::LocalNet);
    match args.manifest.as_ref() {
        Some(manifest) => {
            if args.random_signer && args.signer_secret_key.is_none() {
                anyhow::bail!(
                    "--random-signer requires --signer: a fresh random key seals each transaction, while the --signer \
                     key is added as an additional signer (e.g. it owns the fee-paying account and authorises pay_fee)"
                );
            }
            let signer_key = args
                .signer_secret_key
                .as_ref()
                .map(|s| RistrettoSecretKey::from_hex(s))
                .transpose()
                .map_err(|_| anyhow!("Failed to parse secret"))?
                .unwrap_or_else(|| RistrettoSecretKey::random(&mut rand::rng()));
            let mut manifest_args = parse_args(&args.manifest_args)?;
            if let Some(args_file) = &args.manifest_args_file {
                let file = io::BufReader::new(fs::File::open(args_file)?);
                for ln in file.lines() {
                    let ln = ln?;
                    let line = ln.trim();
                    if line.is_empty() || line.starts_with('#') {
                        continue;
                    }
                    manifest_args.extend(parse_arg(line));
                }
            }
            let templates = parse_templates(&args.templates)?;
            let inputs = parse_inputs(&args.inputs)?;
            let blobs = parse_blobs(&args.blobs)?;
            manifest::builder(
                signer_key,
                network,
                manifest,
                manifest_args,
                templates,
                inputs,
                blobs,
                args.random_signer,
                max_epoch,
            )
        },
        None => match args.flood {
            Flood::Faucet => match args.account_seed {
                Some(seed) => Ok(Box::new(free_coins::seeded_builder(network, max_epoch, seed))),
                None => Ok(Box::new(free_coins::builder(network, max_epoch))),
            },
            Flood::Transfers => {
                let seed = args.account_seed.ok_or_else(|| {
                    anyhow!(
                        "--flood transfers spends from the accounts a faucet flood funded: pass the --account-seed \
                         that flood used"
                    )
                })?;
                let vaults = resolve_account_vaults(&args.indexer_url, seed, args.num_transactions)?;
                Ok(Box::new(transfers::builder(network, max_epoch, seed, vaults)))
            },
        },
    }
}

/// The TARI vault of each of the first `count` accounts derived from `seed`, in account order.
///
/// A vault id is allocated from the hash of the transaction that created it, so it is read from the indexer rather
/// than derived. A faucet-funded account holds exactly one vault.
fn resolve_account_vaults(indexer_url: &Url, seed: u64, count: u64) -> anyhow::Result<Vec<VaultId>> {
    let client = IndexerRestApiClient::connect(indexer_url.clone())?;
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    let indices = (0..count).collect::<Vec<_>>();
    let mut vaults = Vec::with_capacity(indices.len());
    for chunk in indices.chunks(MAX_SUBSTATES_PER_FETCH) {
        let addresses = chunk
            .iter()
            .map(|&index| account_address(&account_keypair(seed, index).1))
            .collect::<Vec<_>>();
        let requests = BoundedVec::<_, 1, 50>::from_vec(
            addresses
                .iter()
                .map(|&address| SubstateId::Component(address))
                .collect(),
        )
        .map_err(|e| anyhow!("Could not batch account lookups: {e}"))?;
        let response = runtime
            .block_on(client.fetch_substates(GetSubstatesRequest {
                requests,
                cached_only: false,
                include_proofs: false,
            }))
            .map_err(|e| anyhow!("Could not read the accounts of seed {seed} from indexer {indexer_url}: {e}"))?;

        for (&index, address) in chunk.iter().zip(&addresses) {
            let value = response
                .substates
                .get(&SubstateId::Component(*address))
                .map(|substate| substate.value().decode())
                .transpose()
                .map_err(|e| anyhow!("Could not decode account {index} of seed {seed} ({address}): {e}"))?;
            let component = value.as_ref().and_then(|value| value.component()).ok_or_else(|| {
                anyhow!(
                    "Account {index} of seed {seed} ({address}) does not exist. Fund the accounts first with \
                     `--account-seed {seed}` and at least {count} faucet transactions."
                )
            })?;
            let indexed = component
                .body
                .to_indexed_well_known_types()
                .map_err(|e| anyhow!("Could not read the vaults of account {index} ({address}): {e}"))?;
            match indexed.vault_ids() {
                [vault] => vaults.push(*vault),
                found => anyhow::bail!(
                    "Account {index} of seed {seed} ({address}) holds {} vaults, but a transfers flood spends only \
                     the TARI vault the faucet funded",
                    found.len()
                ),
            }
        }
    }
    Ok(vaults)
}

fn parse_inputs(items: &[String]) -> anyhow::Result<Vec<InputDeclaration>> {
    items
        .iter()
        .map(|s| {
            s.trim()
                .parse::<InputDeclaration>()
                .map_err(|e| anyhow!("Invalid --input '{}': {}", s, e))
        })
        .collect()
}

fn parse_blobs(items: &[String]) -> anyhow::Result<HashMap<String, Blob>> {
    items
        .iter()
        .map(|s| {
            let (name, path) = s
                .split_once('=')
                .ok_or_else(|| anyhow!("Invalid --blob mapping '{}' (expected <name>=<file_path>)", s))?;
            let name = name.trim();
            let path = path.trim();
            if name.is_empty() || path.is_empty() {
                anyhow::bail!("Invalid --blob mapping '{}' (expected <name>=<file_path>)", s);
            }
            let bytes =
                fs::read(path).map_err(|e| anyhow!("Failed to read blob '{}' from file '{}': {}", name, path, e))?;
            Ok((name.to_string(), Blob::from(bytes)))
        })
        .collect()
}

fn parse_templates(items: &[String]) -> anyhow::Result<HashMap<String, TemplateAddress>> {
    items
        .iter()
        .map(|s| {
            let (alias, address) = s
                .split_once('=')
                .ok_or_else(|| anyhow!("Invalid template mapping '{}' (expected <alias>=template_<hex>)", s))?;
            let trimmed = address.trim();
            let hex = trimmed.strip_prefix("template_").unwrap_or(trimmed).trim();
            let address = TemplateAddress::from_hex(hex)
                .map_err(|_| anyhow!("Invalid template address for '{}': {}", alias, trimmed))?;
            Ok((alias.trim().to_string(), address))
        })
        .collect()
}

fn parse_args(globals: &[String]) -> Result<HashMap<String, ManifestValue>, anyhow::Error> {
    globals.iter().map(|s| parse_arg(s)).collect()
}

fn parse_arg(arg: &str) -> Result<(String, ManifestValue), anyhow::Error> {
    let (name, value) = arg.split_once('=').ok_or_else(|| anyhow!("Invalid arg: {}", arg))?;
    let value = value
        .trim()
        .parse()
        .map_err(|err| anyhow!("Failed to parse arg '{}': {}", name, err))?;
    Ok((name.trim().to_string(), value))
}
