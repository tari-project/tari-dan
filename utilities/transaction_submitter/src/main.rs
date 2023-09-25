//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    cmp,
    fs::File,
    io::Write,
    sync::{atomic::AtomicUsize, Arc},
};

use anyhow::bail;
use tari_validator_node_client::{types::SubmitTransactionRequest, ValidatorNodeClient};
use tokio::task;
use transaction_generator::{read_number_of_transactions, read_transactions};

use crate::{
    bounded_spawn::BoundedSpawn,
    cli::{Cli, StressTestArgs, SubCommand},
};
mod cli;

pub mod bounded_spawn;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::init();
    match cli.sub_command {
        SubCommand::StressTest(args) => {
            stress_test(args).await?;
        },
    }

    Ok(())
}

async fn stress_test(args: StressTestArgs) -> anyhow::Result<()> {
    let mut clients = Vec::with_capacity(args.jrpc_address.len());
    for address in args.jrpc_address {
        let mut client = ValidatorNodeClient::connect(format!("http://{}/json_rpc", address))?;
        if let Err(e) = client.get_identity().await {
            bail!("Failed to connect to {}: {}", address, e);
        }
        clients.push(client);
    }

    let num_transactions = read_number_of_transactions(&mut File::open(&args.transaction_file)?)?;

    println!(
        "{} contains {} transactions",
        args.transaction_file.display(),
        num_transactions
    );
    if args
        .num_transactions
        .map(|n| n + args.skip_transactions.unwrap_or(0) > num_transactions)
        .unwrap_or(false)
    {
        bail!(
            "The transaction file only contains {} transactions, but you requested {}",
            num_transactions,
            args.num_transactions.unwrap_or(num_transactions) + args.skip_transactions.unwrap_or(0)
        );
    }
    let num_transactions = cmp::min(num_transactions, args.num_transactions.unwrap_or(num_transactions));
    if !args.no_confirm {
        print!("{} transactions will be submitted. Continue? [y/N]: ", num_transactions);
        std::io::stdout().flush()?;
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Aborting");
            return Ok(());
        }
    }

    println!("⚠️ Submitting {} transactions", num_transactions);

    if num_transactions == 0 {
        return Ok(());
    }

    let transactions = read_transactions(File::open(args.transaction_file)?, args.skip_transactions.unwrap_or(0))?;

    let mut count = 0usize;
    let bounded_spawn = BoundedSpawn::new(clients.len() * 100);
    let task_counter = Arc::new(AtomicUsize::new(0));
    while let Ok(transaction) = transactions.recv() {
        let mut client = clients[count % clients.len()].clone();
        task_counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        // Bounded spawn prevents too many tasks from being spawned at once, to prevent opening too many sockets in the
        // OS.
        bounded_spawn
            .spawn({
                let counter = task_counter.clone();
                async move {
                    if let Err(e) = client
                        .submit_transaction(SubmitTransactionRequest {
                            transaction,
                            is_dry_run: false,
                        })
                        .await
                    {
                        println!("Failed to submit transaction: {}", e);
                    }
                    counter.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                }
            })
            .await;

        count += 1;
        if num_transactions <= count as u64 {
            break;
        }
    }

    // Kinda hacky, but there will still be tasks waiting in the queue at this point so we can't quit yet
    while task_counter.load(std::sync::atomic::Ordering::SeqCst) > 0 {
        task::yield_now().await;
    }

    Ok(())
}
