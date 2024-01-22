//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{cmp, fs::File, io::Write, time::Duration};

use anyhow::bail;
use tari_transaction::TransactionId;
use tari_validator_node_client::{
    types::{GetTransactionResultRequest, SubmitTransactionRequest, SubmitTransactionResponse},
    ValidatorNodeClient,
};
use tokio::{
    sync::mpsc,
    task,
    time::{sleep, timeout},
};
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
            if let Some(summary) = stress_test(args).await? {
                print_summary(&summary);
            }
        },
    }

    Ok(())
}

async fn stress_test(args: StressTestArgs) -> anyhow::Result<Option<StressTestResultSummary>> {
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
            return Ok(None);
        }
    }

    println!("⚠️ Submitting {} transactions", num_transactions);

    if num_transactions == 0 {
        return Ok(Some(StressTestResultSummary::default()));
    }

    let transactions = read_transactions(File::open(args.transaction_file)?, args.skip_transactions.unwrap_or(0))?;

    let mut count = 0usize;
    let bounded_spawn = BoundedSpawn::new(clients.len() * 100);
    let (submitted_tx, submitted_rx) = mpsc::unbounded_channel();
    while let Ok(transaction) = transactions.recv() {
        let mut client = clients[count % clients.len()].clone();
        let submitted_tx = submitted_tx.clone();

        // Bounded spawn prevents too many tasks from being spawned at once, to prevent opening too many sockets in the
        // OS.
        bounded_spawn
            .spawn(async move {
                match client
                    .submit_transaction(SubmitTransactionRequest {
                        transaction,
                        is_dry_run: false,
                    })
                    .await
                {
                    Ok(SubmitTransactionResponse { transaction_id, .. }) => {
                        submitted_tx.send(transaction_id).unwrap();
                    },
                    Err(e) => {
                        println!("Failed to submit transaction: {}", e);
                    },
                }
            })
            .await;

        count += 1;
        if num_transactions <= count as u64 {
            break;
        }
    }

    // Drop the remaining sender handle so that the result emitter ends when all results have been received
    drop(submitted_tx);

    println!("Fetching results for {} transactions...", count);
    let results = fetch_result_summary(clients, submitted_rx).await;

    Ok(Some(results))
}

async fn fetch_result_summary(
    clients: Vec<ValidatorNodeClient>,
    mut submitted_rx: mpsc::UnboundedReceiver<TransactionId>,
) -> StressTestResultSummary {
    let bounded_spawn = BoundedSpawn::new(clients.len());
    let mut count = 0;
    let (results_tx, mut results_rx) = mpsc::channel::<TxFinalized>(10);

    // Result collector
    let results_handle = task::spawn(async move {
        let mut result = StressTestResultSummary::default();
        loop {
            match timeout(Duration::from_secs(10), results_rx.recv()).await {
                Ok(Some(tx)) => {
                    result.num_transactions += 1;
                    if tx.is_committed {
                        result.num_committed += 1;
                        result.num_up_substates += tx.num_up_substates;
                        result.num_down_substates += tx.num_down_substates;
                        result.slowest_execution_time = cmp::max(result.slowest_execution_time, tx.execution_time);
                        result.fastest_execution_time = cmp::min(result.fastest_execution_time, tx.execution_time);
                        result.total_execution_time += tx.execution_time;
                    }
                    if tx.is_error {
                        result.num_errors += 1;
                    }
                },
                Ok(None) => break,
                Err(_) => {
                    println!("Still waiting for a result after 10s...");
                    if result.num_transactions > 0 {
                        println!("Results so far:");
                        print_summary(&result);
                        println!();
                    }
                },
            }
        }
        result
    });

    // Result emitter
    while let Some(transaction_id) = submitted_rx.recv().await {
        let mut client = clients[count % clients.len()].clone();
        let results_tx = results_tx.clone();
        bounded_spawn
            .spawn(async move {
                loop {
                    match client
                        .get_transaction_result(GetTransactionResultRequest { transaction_id })
                        .await
                    {
                        Ok(result) => {
                            if result.is_finalized {
                                let result = if let Some(diff) = result.result.unwrap().finalize.result.accept() {
                                    TxFinalized {
                                        is_committed: true,
                                        is_error: false,
                                        num_up_substates: diff.up_len(),
                                        num_down_substates: diff.down_len(),
                                        execution_time: result.execution_time.unwrap(),
                                    }
                                } else {
                                    TxFinalized {
                                        is_committed: false,
                                        is_error: false,
                                        num_up_substates: 0,
                                        num_down_substates: 0,
                                        execution_time: result.execution_time.unwrap(),
                                    }
                                };

                                results_tx.send(result).await.unwrap();
                                break;
                            } else {
                                sleep(Duration::from_secs(1)).await;
                            }
                        },
                        Err(e) => {
                            println!("Failed to get transaction result: {}", e);
                            results_tx
                                .send(TxFinalized {
                                    is_committed: false,
                                    is_error: true,
                                    num_up_substates: 0,
                                    num_down_substates: 0,
                                    execution_time: Duration::from_secs(0),
                                })
                                .await
                                .unwrap();
                            break;
                        },
                    }
                }
            })
            .await;

        count += 1;
    }

    // Drop the remaining sender handle so that the result collector ends when all results have been received
    drop(results_tx);
    results_handle.await.unwrap()
}

struct TxFinalized {
    pub is_committed: bool,
    pub is_error: bool,
    pub num_up_substates: usize,
    pub num_down_substates: usize,
    pub execution_time: Duration,
}

#[derive(Debug, Clone)]
pub struct StressTestResultSummary {
    pub num_transactions: usize,
    pub num_committed: usize,
    pub num_errors: usize,
    pub num_up_substates: usize,
    pub num_down_substates: usize,
    pub slowest_execution_time: Duration,
    pub fastest_execution_time: Duration,
    pub total_execution_time: Duration,
}

impl Default for StressTestResultSummary {
    fn default() -> Self {
        Self {
            num_transactions: 0,
            num_committed: 0,
            num_errors: 0,
            num_up_substates: 0,
            num_down_substates: 0,
            slowest_execution_time: Duration::from_secs(0),
            fastest_execution_time: Duration::MAX,
            total_execution_time: Duration::from_secs(0),
        }
    }
}

fn print_summary(summary: &StressTestResultSummary) {
    println!("Summary:");
    println!(
        "  Success rate: {:.2}%",
        summary.num_committed as f64 / summary.num_transactions as f64 * 100.0
    );
    println!("  Transactions submitted: {}", summary.num_transactions);
    println!("  Transactions committed: {}", summary.num_committed);
    println!("  Transactions errored: {}", summary.num_errors);
    println!("  Up substates: {}", summary.num_up_substates);
    println!("  Down substates: {}", summary.num_down_substates);
    println!(
        "  Total execution time: {:.2?} (slowest: {:.2?}, fastest: {:.2?})",
        summary.total_execution_time, summary.slowest_execution_time, summary.fastest_execution_time
    );

    println!(
        "  Avg execution time: {}",
        summary
            .total_execution_time
            .as_millis()
            .checked_div(summary.num_committed as u128)
            .map(|n| format!("{}ms", n))
            .unwrap_or_else(|| "--".to_string())
    );
}
