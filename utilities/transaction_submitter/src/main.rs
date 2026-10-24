//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{cmp, fs::File, io::Write, path::Path, time::Duration};

use anyhow::bail;
use tari_indexer_client::{
    rest_api_client::IndexerRestApiClient,
    types::{
        GetTransactionResultRequest,
        GetTransactionResultResponse,
        IndexerTransactionFinalizedResult,
        SubmitTransactionRequest,
        SubmitTransactionResponse,
    },
};
use tari_ootle_common_types::optional::Optional;
use tari_ootle_transaction::{TransactionEnvelope, TransactionId};
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
    if args.indexer_urls.is_empty() {
        bail!("No indexer URLs specified");
    }
    let mut clients = Vec::with_capacity(args.indexer_urls.len());
    for url in &args.indexer_urls {
        let endpoint = normalize_endpoint(url);
        let client = IndexerRestApiClient::connect(endpoint.clone())?;
        if let Err(e) = client.get_network_info().await {
            bail!("Failed to connect to {}: {}", endpoint, e);
        }
        clients.push(IndexerClient {
            client,
            endpoint: endpoint.clone(),
        });
    }

    let num_transactions = read_number_of_transactions(&mut File::open(&args.transaction_file)?)?;

    println!(
        "{} contains {} transactions",
        args.transaction_file.display(),
        num_transactions
    );
    if args
        .num_transactions
        .map(|n| n.saturating_add(args.skip_transactions.unwrap_or(0)) > num_transactions)
        .unwrap_or(false)
    {
        bail!(
            "The transaction file only contains {} transactions, but you requested {}",
            num_transactions,
            args.num_transactions
                .unwrap_or(num_transactions)
                .saturating_add(args.skip_transactions.unwrap_or(0))
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

    check_validity_window(&clients[0], &args.transaction_file, args.skip_transactions.unwrap_or(0)).await?;

    let transactions = read_transactions(File::open(args.transaction_file)?, args.skip_transactions.unwrap_or(0))?;

    let mut count = 0usize;
    let bounded_spawn = BoundedSpawn::new(clients.len() * 100);
    let (submitted_tx, submitted_rx) = mpsc::unbounded_channel();
    while let Ok(transaction) = transactions.recv() {
        let client = clients[count % clients.len()].clone();
        let submitted_tx = submitted_tx.clone();

        // Bounded spawn prevents too many tasks from being spawned at once, to prevent opening too many sockets in the
        // OS.
        bounded_spawn
            .spawn(async move {
                let envelope = match TransactionEnvelope::encode(transaction) {
                    Ok(env) => env,
                    Err(e) => {
                        println!("Failed to encode transaction: {}", e);
                        return;
                    },
                };
                match client
                    .client
                    .submit_transaction(SubmitTransactionRequest { transaction: envelope })
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

#[allow(clippy::too_many_lines)]
async fn fetch_result_summary(
    clients: Vec<IndexerClient>,
    mut submitted_rx: mpsc::UnboundedReceiver<TransactionId>,
) -> StressTestResultSummary {
    let bounded_spawn = BoundedSpawn::new(clients.len());
    let (results_tx, mut results_rx) = mpsc::channel::<TxFinalized>(10);

    // Result collector
    let results_handle = task::spawn(async move {
        let mut result = StressTestResultSummary::default();
        loop {
            match timeout(Duration::from_secs(10), results_rx.recv()).await {
                Ok(Some(tx)) => {
                    result.num_transactions += 1;
                    match tx.outcome {
                        Outcome::Committed => {
                            result.num_committed += 1;
                            result.num_up_substates += tx.num_up_substates;
                            result.num_down_substates += tx.num_down_substates;
                            result.record_execution_time(tx.execution_time);
                        },
                        // Still executed (and consumed validator time), so it counts toward the
                        // execution-time stats — just not as a committed success.
                        Outcome::FeeAcceptedExecutionRejected => {
                            result.num_execution_rejected += 1;
                            result.record_execution_time(tx.execution_time);
                        },
                        Outcome::Rejected => result.num_rejected += 1,
                        Outcome::Error => result.num_errors += 1,
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
        let clients = clients.clone();
        let num_clients = clients.len();
        let results_tx = results_tx.clone();
        bounded_spawn
            .spawn(async move {
                let mut i = 0usize;
                loop {
                    let client = &clients[i % num_clients];
                    i += 1;
                    match client
                        .client
                        .get_transaction_result(GetTransactionResultRequest {
                            transaction_id,
                            include_proof: false,
                        })
                        .await
                        .optional()
                    {
                        Ok(Some(GetTransactionResultResponse {
                            result:
                                IndexerTransactionFinalizedResult::Finalized {
                                    execution_result,
                                    execution_time,
                                    ..
                                },
                            ..
                        })) => {
                            let execution_result = execution_result
                                .map(|r| r.decode())
                                .transpose()
                                .expect("indexer served an execution result that does not decode");
                            let result = match execution_result {
                                // Full accept: fee and main intents both committed.
                                Some(exec_result) if exec_result.finalize.is_full_accept() => {
                                    let diff = exec_result.finalize.accept().expect("is_full_accept");
                                    TxFinalized {
                                        outcome: Outcome::Committed,
                                        num_up_substates: diff.up_len(),
                                        num_down_substates: diff.down_len(),
                                        execution_time,
                                    }
                                },
                                // Fee charged but the main body was rejected — e.g. it ran out of the
                                // per-transaction metering budget. The transaction executed but did not
                                // do its intended work, so it is neither a full success nor an error.
                                Some(exec_result) if exec_result.finalize.is_fee_only() => TxFinalized {
                                    outcome: Outcome::FeeAcceptedExecutionRejected,
                                    num_up_substates: 0,
                                    num_down_substates: 0,
                                    execution_time,
                                },
                                // Rejected outright (or finalized with no execution result).
                                _ => TxFinalized {
                                    outcome: Outcome::Rejected,
                                    num_up_substates: 0,
                                    num_down_substates: 0,
                                    execution_time,
                                },
                            };

                            results_tx.send(result).await.unwrap();
                            break;
                        },
                        Ok(Some(GetTransactionResultResponse {
                            result: IndexerTransactionFinalizedResult::Pending,
                            ..
                        })) => {
                            sleep(Duration::from_secs(1)).await;
                        },
                        Ok(Some(GetTransactionResultResponse {
                            result: IndexerTransactionFinalizedResult::Rejected { details, .. },
                            ..
                        })) => {
                            println!(
                                "Transaction {} rejected by mempool validation: {}",
                                transaction_id, details
                            );
                            results_tx
                                .send(TxFinalized {
                                    outcome: Outcome::Rejected,
                                    num_up_substates: 0,
                                    num_down_substates: 0,
                                    execution_time: Duration::from_secs(0),
                                })
                                .await
                                .unwrap();
                            break;
                        },
                        Ok(None) => {
                            println!(
                                "[{}] Result not found for transaction {}. The indexer may not have seen it yet. \
                                 Retrying later...",
                                client.endpoint, transaction_id
                            );
                            sleep(Duration::from_secs(1)).await;
                        },
                        Err(e) => {
                            println!("Failed to get transaction result: {}", e);
                            results_tx
                                .send(TxFinalized {
                                    outcome: Outcome::Error,
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
    }

    // Drop the remaining sender handle so that the result collector ends when all results have been received
    drop(results_tx);
    results_handle.await.unwrap()
}

#[derive(Clone)]
struct IndexerClient {
    client: IndexerRestApiClient,
    endpoint: String,
}

fn normalize_endpoint(input: &str) -> String {
    let mut url = if input.starts_with("http://") || input.starts_with("https://") {
        input.to_string()
    } else {
        format!("http://{input}")
    };
    if !url.ends_with('/') {
        url.push('/');
    }
    url
}

/// How a submitted transaction was finalized.
enum Outcome {
    /// Full accept — fee and main intents both committed.
    Committed,
    /// Fee was charged but the main body was rejected (e.g. it ran out of the per-transaction
    /// metering budget). The transaction executed but did not do its intended work.
    FeeAcceptedExecutionRejected,
    /// Rejected outright (no accepted diff), or finalized with no execution result.
    Rejected,
    /// Could not be retrieved / errored while fetching the result.
    Error,
}

struct TxFinalized {
    outcome: Outcome,
    num_up_substates: usize,
    num_down_substates: usize,
    execution_time: Duration,
}

#[derive(Debug, Clone)]
pub struct StressTestResultSummary {
    pub num_transactions: usize,
    pub num_committed: usize,
    /// Fee charged but the main body rejected .
    pub num_execution_rejected: usize,
    /// Rejected outright (no accepted diff).
    pub num_rejected: usize,
    pub num_errors: usize,
    pub num_up_substates: usize,
    pub num_down_substates: usize,
    pub slowest_execution_time: Duration,
    pub fastest_execution_time: Duration,
    pub total_execution_time: Duration,
}

impl StressTestResultSummary {
    fn record_execution_time(&mut self, execution_time: Duration) {
        self.slowest_execution_time = cmp::max(self.slowest_execution_time, execution_time);
        self.fastest_execution_time = cmp::min(self.fastest_execution_time, execution_time);
        self.total_execution_time += execution_time;
    }

    /// Transactions that actually executed (committed or fee-charged-then-rejected).
    fn num_executed(&self) -> usize {
        self.num_committed + self.num_execution_rejected
    }
}

impl Default for StressTestResultSummary {
    fn default() -> Self {
        Self {
            num_transactions: 0,
            num_committed: 0,
            num_execution_rejected: 0,
            num_rejected: 0,
            num_errors: 0,
            num_up_substates: 0,
            num_down_substates: 0,
            slowest_execution_time: Duration::from_secs(0),
            fastest_execution_time: Duration::MAX,
            total_execution_time: Duration::from_secs(0),
        }
    }
}

/// Fails before submitting anything if the file's transactions can no longer be sequenced.
///
/// `max_epoch` is signed over, so it is fixed when the file is generated and the submitter cannot
/// refresh it. Every transaction in a file is built the same way, so the first one answers for all
/// of them — and checking once turns a run of N indistinguishable per-transaction rejections into a
/// single actionable error.
async fn check_validity_window(client: &IndexerClient, transaction_file: &Path, skip: u64) -> anyhow::Result<()> {
    let transactions = read_transactions(File::open(transaction_file)?, skip)?;
    let Ok(first) = transactions.recv() else {
        return Ok(());
    };

    let current_epoch = client.client.get_epoch_manager_stats().await?.current_epoch;
    let max_epoch = first.max_epoch();
    if current_epoch > max_epoch {
        bail!(
            "The transactions in {} expired at epoch {max_epoch} and {} is now at epoch {current_epoch}. They can \
             never be sequenced — regenerate the file.",
            transaction_file.display(),
            client.endpoint,
        );
    }

    Ok(())
}

fn print_summary(summary: &StressTestResultSummary) {
    println!("Summary:");
    println!(
        "  Success rate: {:.2}%",
        summary.num_committed as f64 / summary.num_transactions as f64 * 100.0
    );
    println!("  Transactions submitted: {}", summary.num_transactions);
    println!("  Fully committed: {}", summary.num_committed);
    println!("  Fee charged, execution rejected: {}", summary.num_execution_rejected);
    println!("  Rejected: {}", summary.num_rejected);
    println!("  Errored: {}", summary.num_errors);
    println!("  Up substates: {}", summary.num_up_substates);
    println!("  Down substates: {}", summary.num_down_substates);

    let avg = summary
        .total_execution_time
        .as_nanos()
        .checked_div(summary.num_executed() as u128)
        .map(|n| Duration::from_nanos(n.try_into().unwrap_or(u64::MAX)))
        .map(|n| format!("{:.2?}", n))
        .unwrap_or_else(|| "--".to_string());

    println!(
        "  Execution time over {} executed: total {:.2?} (slowest: {:.2?}, fastest: {:.2?}, Avg: {avg})",
        summary.num_executed(),
        summary.total_execution_time,
        summary.slowest_execution_time,
        summary.fastest_execution_time
    );
}
