//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

mod cli;
mod transaction_writer;

use std::{
    collections::HashMap,
    fs,
    io,
    io::{stdout, BufRead, Seek, SeekFrom, Write},
};

use anyhow::anyhow;
use cli::Cli;
use rand::rngs::OsRng;
use tari_crypto::{keys::SecretKey, ristretto::RistrettoSecretKey, tari_utilities::hex::Hex};
use tari_transaction_manifest::ManifestValue;
use transaction_generator::{
    read_number_of_transactions,
    read_transactions,
    transaction_builders::{free_coins, manifest},
    BoxedTransactionBuilder,
};

use crate::{
    cli::{SubCommand, WriteArgs},
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

            let builder = get_transaction_builder(&args)?;
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
                println!("Read transaction: {}", transaction.id());
            }
        },
    }

    Ok(())
}

fn get_transaction_builder(args: &WriteArgs) -> anyhow::Result<BoxedTransactionBuilder> {
    match args.manifest.as_ref() {
        Some(manifest) => {
            let signer_key = args
                .signer_secret_key
                .as_ref()
                .map(|s| RistrettoSecretKey::from_hex(s))
                .transpose()
                .map_err(|_| anyhow!("Failed to parse secret"))?
                .unwrap_or_else(|| RistrettoSecretKey::random(&mut OsRng));
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
            manifest::builder(signer_key, manifest, manifest_args, HashMap::new())
        },
        None => Ok(Box::new(free_coins::builder)),
    }
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
