//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

mod cli;
mod transaction_writer;

use std::{
    collections::HashMap,
    fs,
    io::{stdout, Seek, SeekFrom, Write},
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
            manifest::builder(signer_key, manifest, parse_globals(&args.manifest_globals)?)
        },
        None => Ok(Box::new(free_coins::builder)),
    }
}

fn parse_globals(globals: &[String]) -> Result<HashMap<String, ManifestValue>, anyhow::Error> {
    let mut result = HashMap::with_capacity(globals.len());
    for global in globals {
        let (name, value) = global
            .split_once('=')
            .ok_or_else(|| anyhow!("Invalid global: {}", global))?;
        let value = value
            .parse()
            .map_err(|err| anyhow!("Failed to parse global '{}': {}", name, err))?;
        result.insert(name.to_string(), value);
    }
    Ok(result)
}
