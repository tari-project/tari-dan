//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

mod cli;
mod transaction_writer;

use std::io::{stdout, Seek, SeekFrom, Write};

use cli::Cli;
use tari_crypto::ristretto::RistrettoSecretKey;
use tari_template_lib::models::Amount;
use transaction_generator::{read_number_of_transactions, read_transactions};

use crate::{cli::SubCommand, transaction_writer::write_transactions};

fn main() -> anyhow::Result<()> {
    let cli = Cli::init();
    match cli.sub_command {
        SubCommand::Write(args) => {
            let fee_amount = Amount(1000);
            let signer_private_key = RistrettoSecretKey::default();

            if !args.overwrite && args.output_file.exists() {
                anyhow::bail!("Output file {} already exists", args.output_file.display());
            }

            let timer = std::time::Instant::now();
            println!("Generating and writing {} transactions", args.num_transactions,);

            let mut file = std::fs::File::create(&args.output_file)?;
            write_transactions(
                args.num_transactions,
                signer_private_key,
                fee_amount,
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
            let mut file = std::fs::File::open(args.input_file)?;

            let num_transactions = read_number_of_transactions(&mut file)?;
            println!("Number of transactions: {}", num_transactions);
            file.seek(SeekFrom::Start(0))?;
            let receiver = read_transactions(file)?;

            while let Ok(transaction) = receiver.recv() {
                println!("Read transaction: {}", transaction.id());
            }
        },
    }

    Ok(())
}
