//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{BTreeMap, HashMap},
    future::Future,
    mem,
    pin::{Pin, pin},
    sync::{Arc, Weak},
    task::{Context, Poll},
    time::{Duration, Instant},
};

use futures::{FutureExt, StreamExt, stream::BoxStream};
use tari_indexer_client::{
    error::IndexerRestClientError,
    event::TransactionFinalizedEvent,
    rest_api_client::IndexerRestApiClient,
    sse,
    types::{GetTransactionResultRequest, IndexerTransactionFinalizedResult},
};
use tari_ootle_common_types::{
    engine_types::{
        commit_result::{ExecuteResult, TransactionResult},
        encoded::Encoded,
        transaction_receipt::{FinalizeOutcome, TransactionReceipt},
    },
    optional::Optional,
};
use tari_ootle_transaction::TransactionId;
use tokio::{
    sync::{mpsc, oneshot},
    task,
    time::{sleep, sleep_until},
};

use crate::{
    TransactionOutcome,
    provider::{
        VerifyError,
        tx_stream::{EventStreamError, Paused},
        verifier::IndexerVerifier,
    },
};

/// How long a pending transaction first waits before it asks the indexer again for a commit it cannot verify yet. Each
/// later wait doubles, up to [`MAX_RESULT_POLL_INTERVAL`].
const MIN_RESULT_POLL_INTERVAL: Duration = Duration::from_millis(500);
const MAX_RESULT_POLL_INTERVAL: Duration = Duration::from_secs(4);

/// How often the watcher drops watches that nothing waits on any more, so that it pauses the event stream soon after
/// the last one goes.
const ABANDONED_WATCH_CHECK_INTERVAL: Duration = Duration::from_secs(1);

pub struct TransactionWatcher {
    stream: BoxStream<'static, Result<sse::Event, EventStreamError>>,
    span: tracing::Span,
    pending_requests: HashMap<TransactionId, TxWatchRequest>,
    reap_times: BTreeMap<Instant, TransactionId>,
    paused: Paused,
}

impl TransactionWatcher {
    pub(crate) fn new(stream: BoxStream<'static, Result<sse::Event, EventStreamError>>, paused: Paused) -> Self {
        let span = tracing::span!(tracing::Level::DEBUG, "TransactionWatcher::new");
        Self {
            stream,
            span,
            pending_requests: HashMap::new(),
            reap_times: BTreeMap::new(),
            paused,
        }
    }

    pub(crate) fn spawn(self) -> TransactionWatcherHandle {
        let (tx_requests, rx_requests) = mpsc::channel(2);
        task::spawn(self.run(rx_requests));
        TransactionWatcherHandle::new(tx_requests)
    }

    /// Returns the next time a transaction watch request is set to be reaped, or checked for being abandoned. With no
    /// watch pending, a time far in the future i.e. don't wake up.
    fn next_reap_time(&self) -> Instant {
        let now = Instant::now();
        let next_reap = self
            .reap_times
            .first_key_value()
            .map(|(instant, _)| *instant)
            .unwrap_or(now + Duration::from_secs(60_000));
        if self.pending_requests.is_empty() {
            return next_reap;
        }
        next_reap.min(now + ABANDONED_WATCH_CHECK_INTERVAL)
    }

    /// Drops the watches whose waiter has gone, such as a watch answered by asking the indexer directly.
    fn drop_abandoned_watches(&mut self) {
        self.pending_requests.retain(|tx_id, request| {
            let waited_on = !request.reply.is_closed();
            if !waited_on {
                tracing::debug!("Dropping the watch of tx_id {tx_id}, which nothing waits on");
            }
            waited_on
        });
    }

    async fn run(mut self, mut rx_requests: mpsc::Receiver<TxWatchRequest>) {
        loop {
            self.update_pause_state();

            let next_reap_time = self.next_reap_time();
            let sleep = pin!(sleep_until(next_reap_time.into()));

            tokio::select! {
                Some(request) =  rx_requests.recv() => {
                    self.handle_request(request).await;
                },

                Some(event) = self.stream.next() => {
                    self.handle_event(event).await;
                }

                // Wake up to reap timed out requests even if there are no other events/requests.
                _ = sleep => {}
            }

            self.reap_timeouts();
            self.drop_abandoned_watches();
        }
    }

    fn reap_timeouts(&mut self) {
        let to_keep = self.reap_times.split_off(&Instant::now());
        let to_reap = mem::replace(&mut self.reap_times, to_keep);
        for tx_id in to_reap.into_values() {
            if let Some(request) = self.pending_requests.remove(&tx_id) {
                tracing::warn!("Transaction watch timed out for tx_id: {}", tx_id);
                let _ignore = request
                    .reply
                    .send(Err(PendingTransactionError::Timeout { tx_id }))
                    .inspect_err(|_| {
                        tracing::debug!("The watch of tx_id {} was dropped before it timed out", tx_id);
                    });
            }
        }
    }

    async fn handle_request(&mut self, request: TxWatchRequest) {
        let _enter = self.span.enter();
        tracing::debug!("Received watch request for tx_id: {}", request.tx_id);
        // A timeout too large to represent as an `Instant` never reaps.
        match Instant::now().checked_add(request.timeout) {
            Some(reap_at) => {
                self.reap_times.insert(reap_at, request.tx_id);
            },
            None => {
                tracing::warn!(
                    "Watch request for tx_id {} has an unrepresentable timeout",
                    request.tx_id
                );
            },
        }
        self.pending_requests.insert(request.tx_id, request);
    }

    async fn handle_event(&mut self, event: Result<sse::Event, EventStreamError>) {
        const TX_FINALIZED_EVENT_TYPE: &str = "TransactionFinalized";
        let _enter = self.span.enter();
        match event {
            Ok(event) => {
                tracing::debug!("Received SSE event: {:?}", event);
                if event.event_type != TX_FINALIZED_EVENT_TYPE {
                    tracing::debug!("Ignoring non-finalized event of type: {}", event.event_type);
                    return;
                }

                // Parse the event data to extract transaction ID and outcome
                let event_data: TransactionFinalizedEvent = match serde_json::from_str(&event.data) {
                    Ok(data) => data,
                    Err(e) => {
                        tracing::error!("Failed to parse event data: {}", e);
                        return;
                    },
                };

                let Some(watch) = self.pending_requests.remove(&event_data.transaction_id) else {
                    tracing::debug!("No pending watch found for tx_id: {}", event_data.transaction_id);
                    return;
                };

                tracing::info!(
                    "Transaction {} finalized with outcome: {:?}",
                    event_data.transaction_id,
                    event_data.outcome
                );

                watch.reply.send(Ok(event_data.outcome)).unwrap_or_else(|_| {
                    tracing::debug!(
                        "The watch of tx_id {} was dropped before it finalized",
                        event_data.transaction_id
                    );
                });
            },
            Err(e) => {
                tracing::error!("Error receiving SSE event: {}", e);
            },
        }
    }

    fn update_pause_state(&self) {
        let should_pause = self.pending_requests.is_empty();
        if self.paused.set_paused(should_pause) {
            if should_pause {
                tracing::debug!("No pending transactions. Pausing event stream.");
            } else {
                tracing::debug!("Pending transactions exist. Resuming event stream.");
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PendingTransactionError {
    #[error("Client has been dropped")]
    ClientDropped,
    #[error("Indexer client error: {0}")]
    IndexerClientError(#[from] IndexerRestClientError),
    #[error("The transaction watch was aborted for tx_id: {tx_id}")]
    WatchAborted { tx_id: TransactionId },
    #[error("Transaction receipt not found for tx_id: {tx_id}")]
    ReceiptNotFound { tx_id: TransactionId },
    #[error("Transaction {tx_id} was rejected: {reason}")]
    TransactionRejected { tx_id: TransactionId, reason: String },
    #[error("Transaction timed out for tx_id: {tx_id}")]
    Timeout { tx_id: TransactionId },
    #[error(transparent)]
    Unverified(#[from] VerifyError),
}

impl PendingTransactionError {
    pub fn is_timeout(&self) -> bool {
        matches!(self, PendingTransactionError::Timeout { .. })
    }

    /// Whether the error is a failed request to the indexer, which a later request may not repeat.
    fn is_indexer_unreachable(&self) -> bool {
        matches!(
            self,
            Self::IndexerClientError(_) |
                Self::Unverified(VerifyError::IndexerClientError(_) | VerifyError::CheckpointNotServed { .. })
        )
    }
}

/// The outcome of a transaction whose result committed.
fn committed_outcome(result: TransactionResult) -> TransactionOutcome {
    match result {
        TransactionResult::Accept(_) => TransactionOutcome::Commit,
        TransactionResult::AcceptFeeRejectRest(_, reason) => TransactionOutcome::OnlyFeeCommit(reason),
        TransactionResult::Reject(reason) => TransactionOutcome::Reject(reason),
    }
}

#[derive(Debug)]
pub struct TxWatchRequest {
    pub tx_id: TransactionId,
    pub timeout: Duration,
    pub reply: oneshot::Sender<Result<FinalizeOutcome, PendingTransactionError>>,
}

#[derive(Debug, Clone)]
pub struct TransactionWatcherHandle {
    tx_requests: mpsc::Sender<TxWatchRequest>,
}

impl TransactionWatcherHandle {
    pub fn new(tx_requests: mpsc::Sender<TxWatchRequest>) -> Self {
        Self { tx_requests }
    }

    pub async fn watch_transaction(&self, tx_id: TransactionId, timeout: Duration) -> PendingTransactionOutcome {
        let (tx_reply, rx_reply) = oneshot::channel();
        let request = TxWatchRequest {
            tx_id,
            timeout,
            reply: tx_reply,
        };
        self.tx_requests
            .send(request)
            .await
            .expect("TransactionWatcher not alive");
        PendingTransactionOutcome {
            tx_id,
            outcome_rx: rx_reply,
        }
    }
}

pub struct PendingTransaction {
    watcher: TransactionWatcherHandle,
    client: Weak<IndexerRestApiClient>,
    verifier: Arc<IndexerVerifier>,
    tx_id: TransactionId,
    default_timeout: Duration,
}

/// A transaction's final outcome, with its receipt if it committed.
struct Finalized {
    outcome: TransactionOutcome,
    receipt: Option<TransactionReceipt>,
}

impl PendingTransaction {
    pub(crate) fn new(
        watcher: TransactionWatcherHandle,
        client: Weak<IndexerRestApiClient>,
        verifier: Arc<IndexerVerifier>,
        tx_id: TransactionId,
    ) -> Self {
        const DEFAULT_TX_TIMEOUT: Duration = Duration::from_secs(32);
        Self {
            watcher,
            default_timeout: DEFAULT_TX_TIMEOUT,
            client,
            verifier,
            tx_id,
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.default_timeout = timeout;
        self
    }

    pub fn tx_id(&self) -> TransactionId {
        self.tx_id
    }

    pub async fn register(&self, timeout: Duration) -> Result<PendingTransactionOutcome, PendingTransactionError> {
        let pending = self.watcher.watch_transaction(self.tx_id, timeout).await;
        Ok(pending)
    }

    /// Waits up to the timeout for the transaction to finalize.
    ///
    /// A verifying provider takes the indexer's finalization event only as a hint. It reports a commit once the
    /// indexer proves the transaction's receipt against a validator set the provider trusts, and the result matches
    /// that receipt. No proof shows an abort, so an abort is reported on the indexer's word.
    pub async fn watch(&self) -> Result<TransactionOutcome, PendingTransactionError> {
        if !self.verifier.is_verifying() {
            return self.watch_trusting().await;
        }
        Ok(self.wait_verified().await?.outcome)
    }

    /// Asks the indexer for the verified outcome, and while it has none, waits for the indexer's finalization hint and
    /// asks again until it has one or the timeout passes.
    async fn wait_verified(&self) -> Result<Finalized, PendingTransactionError> {
        let deadline = Instant::now().checked_add(self.default_timeout);
        let hint = self.register(self.default_timeout).await?;
        if let Some(finalized) = self.poll_verified_finalization().await? {
            return Ok(finalized);
        }
        match hint.await {
            Ok(_) | Err(PendingTransactionError::Timeout { .. }) => {},
            Err(err) => return Err(err),
        }
        let mut interval = MIN_RESULT_POLL_INTERVAL;
        loop {
            if let Some(finalized) = self.poll_verified_finalization().await? {
                return Ok(finalized);
            }
            if deadline.is_some_and(|deadline| Instant::now() + interval > deadline) {
                return Err(PendingTransactionError::Timeout { tx_id: self.tx_id });
            }
            sleep(interval).await;
            interval = (interval * 2).min(MAX_RESULT_POLL_INTERVAL);
        }
    }

    /// [`try_verified_finalization`](Self::try_verified_finalization), with a failure to reach the indexer taken as no
    /// outcome yet, so that the next poll asks again.
    async fn poll_verified_finalization(&self) -> Result<Option<Finalized>, PendingTransactionError> {
        match self.try_verified_finalization().await {
            Err(err) if err.is_indexer_unreachable() => {
                tracing::warn!(
                    "Failed to ask the indexer for the outcome of transaction {}: {err}",
                    self.tx_id
                );
                Ok(None)
            },
            result => result,
        }
    }

    /// The transaction's final outcome, verified if it committed. `None` while the transaction is pending, or while
    /// its commit cannot be verified yet.
    async fn try_verified_finalization(&self) -> Result<Option<Finalized>, PendingTransactionError> {
        let client = self.upgrade_client()?;
        let Some(resp) = client
            .get_transaction_result(GetTransactionResultRequest {
                transaction_id: self.tx_id,
                include_proof: true,
            })
            .await
            .optional()?
        else {
            return Ok(None);
        };
        let (final_decision, execution_result, abort_details) = match resp.result {
            IndexerTransactionFinalizedResult::Pending => return Ok(None),
            IndexerTransactionFinalizedResult::Rejected { details, .. } => {
                return Err(PendingTransactionError::TransactionRejected {
                    tx_id: self.tx_id,
                    reason: details,
                });
            },
            IndexerTransactionFinalizedResult::Finalized {
                final_decision,
                execution_result,
                abort_details,
                ..
            } => (final_decision, execution_result, abort_details),
        };
        if !final_decision.is_commit() {
            let execution_result = self.decode_execution_result(execution_result.as_ref())?;
            let outcome = self.abort_outcome(execution_result.as_ref(), abort_details)?;
            return Ok(Some(Finalized { outcome, receipt: None }));
        }
        if execution_result.is_none() {
            return Err(PendingTransactionError::IndexerClientError(
                IndexerRestClientError::InvalidResponse {
                    message: format!("Transaction {} committed, but the indexer served no result", self.tx_id),
                },
            ));
        }
        let Some((receipt, result)) = self
            .verifier
            .verify_committed_result(self.tx_id, resp.receipt.as_ref(), execution_result.as_ref())
            .await?
        else {
            return Ok(None);
        };
        Ok(Some(Finalized {
            outcome: committed_outcome(result.finalize.result),
            receipt: Some(receipt),
        }))
    }

    /// `execution_result` as the indexer served it, decoded.
    fn decode_execution_result(
        &self,
        execution_result: Option<&Encoded<ExecuteResult>>,
    ) -> Result<Option<ExecuteResult>, PendingTransactionError> {
        execution_result
            .map(|encoded| {
                encoded.decode().map_err(|e| {
                    PendingTransactionError::IndexerClientError(IndexerRestClientError::InvalidResponse {
                        message: format!(
                            "The indexer served an execution result of transaction {} that does not decode: {e}",
                            self.tx_id
                        ),
                    })
                })
            })
            .transpose()
    }

    /// The outcome of a transaction that aborted, which commits nothing.
    fn abort_outcome(
        &self,
        execution_result: Option<&ExecuteResult>,
        abort_details: Option<String>,
    ) -> Result<TransactionOutcome, PendingTransactionError> {
        match execution_result.and_then(|result| result.finalize.result.any_reject()) {
            Some(reason) => Ok(TransactionOutcome::Reject(reason.clone())),
            None => Err(PendingTransactionError::TransactionRejected {
                tx_id: self.tx_id,
                reason: abort_details.unwrap_or_else(|| "Unknown".to_string()),
            }),
        }
    }

    async fn watch_trusting(&self) -> Result<TransactionOutcome, PendingTransactionError> {
        match self.register(self.default_timeout).await?.await {
            Ok(FinalizeOutcome::Commit) => Ok(TransactionOutcome::Commit),
            Ok(FinalizeOutcome::FeeIntentCommit) => {
                // The transaction was accepted but the main transaction was not committed. This can occur when the
                // transaction was rejected after the fee intent was committed, or if the transaction is still pending
                // but the fee intent was committed. In either case, we need to query the transaction result to
                // determine the actual outcome.
                tracing::warn!(
                    "Transaction fee intent committed but main transaction not committed for tx_id: {}. Querying \
                     transaction result to determine actual outcome.",
                    self.tx_id
                );
                match self.try_get_transaction_result().await {
                    Ok(Some(IndexerTransactionFinalizedResult::Pending)) => {
                        // Indexer bug?
                        Err(PendingTransactionError::IndexerClientError(
                            IndexerRestClientError::InvalidResponse {
                                message: format!(
                                    "Transaction {} is still pending after fee intent commit. This may indicate an \
                                     indexer issue.",
                                    self.tx_id
                                ),
                            },
                        ))
                    },
                    Ok(Some(IndexerTransactionFinalizedResult::Finalized {
                        final_decision,
                        execution_result,
                        abort_details,
                        ..
                    })) => self.outcome_from_finalized(final_decision.is_commit(), execution_result, abort_details),
                    Ok(Some(IndexerTransactionFinalizedResult::Rejected { details, .. })) => {
                        Err(PendingTransactionError::TransactionRejected {
                            tx_id: self.tx_id,
                            reason: details,
                        })
                    },
                    Ok(None) => {
                        tracing::error!("Transaction result not found after fee reject: {}", self.tx_id);
                        Err(PendingTransactionError::Timeout { tx_id: self.tx_id })
                    },
                    Err(e) => {
                        tracing::error!("Failed to get transaction result after fee reject: {}", e);
                        Err(e)
                    },
                }
            },
            Err(PendingTransactionError::Timeout { .. }) => {
                tracing::warn!("Transaction watch timed out, attempting direct query: {}", self.tx_id);
                match self.try_get_transaction_result().await {
                    Ok(Some(IndexerTransactionFinalizedResult::Pending)) => {
                        tracing::warn!("Transaction is still pending after timeout: {}", self.tx_id);
                        Err(PendingTransactionError::Timeout { tx_id: self.tx_id })
                    },
                    Ok(Some(IndexerTransactionFinalizedResult::Finalized {
                        final_decision,
                        execution_result,
                        abort_details,
                        ..
                    })) => self.outcome_from_finalized(final_decision.is_commit(), execution_result, abort_details),
                    Ok(Some(IndexerTransactionFinalizedResult::Rejected { details, .. })) => {
                        Err(PendingTransactionError::TransactionRejected {
                            tx_id: self.tx_id,
                            reason: details,
                        })
                    },
                    Ok(None) => {
                        tracing::error!("Transaction result not found after timeout: {}", self.tx_id);
                        Err(PendingTransactionError::Timeout { tx_id: self.tx_id })
                    },
                    Err(e) => {
                        tracing::error!("Failed to get transaction result after timeout: {}", e);
                        Err(PendingTransactionError::Timeout { tx_id: self.tx_id })
                    },
                }
            },
            Err(e) => Err(e),
        }
    }

    fn outcome_from_finalized(
        &self,
        committed: bool,
        execution_result: Option<Encoded<ExecuteResult>>,
        abort_details: Option<String>,
    ) -> Result<TransactionOutcome, PendingTransactionError> {
        match self.decode_execution_result(execution_result.as_ref())? {
            Some(result) if committed => Ok(committed_outcome(result.finalize.result)),
            execution_result => self.abort_outcome(execution_result.as_ref(), abort_details),
        }
    }

    async fn try_get_transaction_result(
        &self,
    ) -> Result<Option<IndexerTransactionFinalizedResult>, PendingTransactionError> {
        let client = self.upgrade_client()?;
        let resp = client
            .get_transaction_result(GetTransactionResultRequest {
                transaction_id: self.tx_id,
                include_proof: false,
            })
            .await
            .optional()?;
        Ok(resp.map(|r| r.result))
    }

    async fn try_get_transaction_receipt(&self) -> Result<Option<TransactionReceipt>, PendingTransactionError> {
        let client = self.upgrade_client()?;
        if let Some(receipt) = client
            .get_transaction_receipt(self.tx_id.into_receipt_address())
            .await
            .optional()?
        {
            let receipt = receipt.receipt.decode().map_err(|e| {
                PendingTransactionError::IndexerClientError(IndexerRestClientError::InvalidResponse {
                    message: format!(
                        "The indexer served a receipt of transaction {} that does not decode: {e}",
                        self.tx_id
                    ),
                })
            })?;
            return Ok(Some(receipt));
        }
        Ok(None)
    }

    fn upgrade_client(&self) -> Result<Arc<IndexerRestApiClient>, PendingTransactionError> {
        let client = self.client.upgrade().ok_or(PendingTransactionError::ClientDropped)?;
        Ok(client)
    }

    /// The receipt of the committed transaction, waiting up to the timeout for it to finalize. A verifying provider
    /// returns only a receipt the indexer proves, as [`watch`](Self::watch) describes.
    pub async fn get_receipt(&self) -> Result<TransactionReceipt, PendingTransactionError> {
        if !self.verifier.is_verifying() {
            return self.get_receipt_trusting().await;
        }
        let finalized = self.wait_verified().await?;
        finalized
            .receipt
            .ok_or_else(|| PendingTransactionError::TransactionRejected {
                tx_id: self.tx_id,
                reason: finalized
                    .outcome
                    .reject_reason()
                    .map(|reason| reason.to_string())
                    .unwrap_or_else(|| "Unknown".to_string()),
            })
    }

    async fn get_receipt_trusting(&self) -> Result<TransactionReceipt, PendingTransactionError> {
        if let Some(receipt) = self.try_get_transaction_receipt().await? {
            return Ok(receipt);
        }
        match self.try_get_transaction_result().await? {
            Some(IndexerTransactionFinalizedResult::Rejected { details, .. }) => {
                return Err(PendingTransactionError::TransactionRejected {
                    tx_id: self.tx_id,
                    reason: details,
                });
            },
            Some(IndexerTransactionFinalizedResult::Pending) | None => {},
            Some(IndexerTransactionFinalizedResult::Finalized {
                final_decision,
                abort_details,
                execution_result,
                ..
            }) => {
                let execution_result = self.decode_execution_result(execution_result.as_ref())?;
                if final_decision.is_abort() {
                    let reason = execution_result
                        .as_ref()
                        .and_then(|res| res.finalize.result.any_reject())
                        .map(|reject| reject.to_string())
                        .or(abort_details)
                        .unwrap_or_else(|| "Unknown".to_string());
                    return Err(PendingTransactionError::TransactionRejected {
                        tx_id: self.tx_id,
                        reason,
                    });
                }
                if final_decision.is_commit() {
                    // Transaction committed but receipt not found. This is a due to a race condition where the indexer
                    // may not have indexed the receipt yet.
                    //
                    // TODO: improvements to the indexer may be needed to fully resolve this.
                    tracing::warn!("Transaction committed but receipt not found for tx_id: {}", self.tx_id);
                    // The accept diff carries the receipt substate.
                    return execution_result
                        .as_ref()
                        .and_then(|res| res.finalize.get_transaction_receipt())
                        .cloned()
                        .ok_or_else(|| PendingTransactionError::ReceiptNotFound { tx_id: self.tx_id });
                }
            },
        }

        let _outcome = self.watch_trusting().await?;

        let receipt = self
            .try_get_transaction_receipt()
            .await?
            .ok_or_else(|| PendingTransactionError::ReceiptNotFound { tx_id: self.tx_id })?;

        Ok(receipt)
    }
}

pub struct PendingTransactionOutcome {
    tx_id: TransactionId,
    outcome_rx: oneshot::Receiver<Result<FinalizeOutcome, PendingTransactionError>>,
}

impl Future for PendingTransactionOutcome {
    type Output = Result<FinalizeOutcome, PendingTransactionError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.outcome_rx
            .poll_unpin(cx)
            .map(|res| res.unwrap_or_else(|_| Err(PendingTransactionError::WatchAborted { tx_id: self.tx_id })))
    }
}

impl PendingTransactionOutcome {
    pub fn tx_id(&self) -> TransactionId {
        self.tx_id
    }
}

#[cfg(test)]
mod tests {
    use tari_consensus_types::Decision;
    use tari_indexer_client::types::GetTransactionResultResponse;
    use tari_ootle_common_types::{
        Epoch,
        engine_types::{commit_result::AbortReason, encoded::Encoded, fees::FeeReceipt},
    };
    use tari_ootle_light_client::test_fixtures::{NETWORK, ValidatorSetFixture};

    use super::*;
    use crate::provider::{
        IndexerTrust,
        test_indexer::{TestIndexer, committed_result},
        tx_stream::{EventStream, PauseWaiter},
    };

    const TRANSACTION_ID: TransactionId = TransactionId::new([7; 32]);

    /// Validator sets of epochs 1 to 5.
    fn sets() -> Vec<ValidatorSetFixture> {
        (1..=5).map(ValidatorSetFixture::new).collect()
    }

    /// [`TRANSACTION_ID`], pending on `indexer`. The returned client must outlive the watch.
    fn pending(indexer: &TestIndexer, indexer_trust: IndexerTrust) -> (Arc<IndexerRestApiClient>, PendingTransaction) {
        let (client, pending, _) = watched(indexer, indexer_trust);
        (client, pending)
    }

    /// [`pending`], with a waiter on the pause of the watcher's event stream.
    fn watched(
        indexer: &TestIndexer,
        indexer_trust: IndexerTrust,
    ) -> (Arc<IndexerRestApiClient>, PendingTransaction, PauseWaiter) {
        let client = Arc::new(tari_indexer_client::connect_rest(indexer.url()).unwrap());
        let verifier = Arc::new(IndexerVerifier::new(Arc::downgrade(&client), NETWORK, indexer_trust));
        let paused = Paused::default();
        let pause_waiter = paused.waiter();
        let events = EventStream::new(Arc::downgrade(&client), paused.waiter());
        let watcher = TransactionWatcher::new(Box::pin(events.into_stream()), paused).spawn();
        let pending = PendingTransaction::new(watcher, Arc::downgrade(&client), verifier, TRANSACTION_ID)
            .with_timeout(Duration::from_secs(2));
        (client, pending, pause_waiter)
    }

    /// An indexer whose latest checkpoint names the set of epoch 5, which committed [`TRANSACTION_ID`] with `result`,
    /// and announces the commit.
    async fn indexer_that_committed(edit: impl FnOnce(&mut GetTransactionResultResponse)) -> TestIndexer {
        let indexer = indexer_that_finalized(edit).await;
        indexer.hint_commit(TRANSACTION_ID);
        indexer
    }

    /// An indexer whose latest checkpoint names the set of epoch 5, which committed [`TRANSACTION_ID`] with `result`,
    /// and announces nothing.
    async fn indexer_that_finalized(edit: impl FnOnce(&mut GetTransactionResultResponse)) -> TestIndexer {
        let sets = sets();
        let indexer = TestIndexer::spawn(&sets, 3).await;
        let mut result = committed_result(TRANSACTION_ID, &sets[4]);
        edit(&mut result);
        indexer.set_result(TRANSACTION_ID, result);
        indexer
    }

    fn without_proof(result: &mut GetTransactionResultResponse) {
        result.receipt.as_mut().unwrap().proof = None;
    }

    #[tokio::test]
    async fn a_commit_is_reported_once_its_receipt_verifies() {
        let indexer = indexer_that_committed(|_| {}).await;
        let (_client, pending) = pending(&indexer, IndexerTrust::Verify);

        assert!(pending.watch().await.unwrap().is_commit());
        assert_eq!(pending.get_receipt().await.unwrap().epoch(), Epoch(5));
    }

    #[tokio::test]
    async fn a_commit_whose_receipt_is_not_proven_is_not_reported() {
        let indexer = indexer_that_committed(without_proof).await;
        let (_client, pending) = pending(&indexer, IndexerTrust::Verify);

        let err = pending.watch().await.unwrap_err();
        assert!(err.is_timeout(), "{err}");
    }

    #[tokio::test]
    async fn a_result_other_than_its_proven_receipt_commits_is_refused() {
        let indexer = indexer_that_committed(|result| {
            let IndexerTransactionFinalizedResult::Finalized {
                execution_result: Some(execution_result),
                ..
            } = &mut result.result
            else {
                panic!("not a committed result");
            };
            let mut other = execution_result.decode().unwrap();
            other.finalize.fee_receipt = FeeReceipt::builder().with_total_fees_paid(1).build();
            *execution_result = Encoded::encode(&other).unwrap();
        })
        .await;
        let (_client, pending) = pending(&indexer, IndexerTrust::Verify);

        let err = pending.watch().await.unwrap_err();
        assert!(
            matches!(
                err,
                PendingTransactionError::Unverified(VerifyError::FinalizationUnverified { .. })
            ),
            "{err}"
        );
    }

    #[tokio::test]
    async fn a_trusting_provider_reports_the_announced_commit() {
        let indexer = indexer_that_committed(without_proof).await;
        let (_client, pending) = pending(&indexer, IndexerTrust::Trust);

        assert!(pending.watch().await.unwrap().is_commit());
    }

    #[tokio::test]
    async fn a_failed_request_for_the_result_is_asked_again() {
        let indexer = indexer_that_committed(|_| {}).await;
        indexer.fail_result_requests(1);
        let (_client, pending) = pending(&indexer, IndexerTrust::Verify);

        assert!(pending.watch().await.unwrap().is_commit());
    }

    #[tokio::test]
    async fn a_transaction_finalized_before_the_watch_is_reported_without_waiting_for_the_announcement() {
        let indexer = indexer_that_finalized(|_| {}).await;
        let (_client, pending) = pending(&indexer, IndexerTrust::Verify);

        let started = Instant::now();
        assert!(pending.watch().await.unwrap().is_commit());
        assert!(started.elapsed() < Duration::from_secs(1), "{:?}", started.elapsed());
    }

    #[tokio::test]
    async fn a_receipt_proven_before_the_anchor_is_refused() {
        let sets = sets();
        let indexer = indexer_that_committed(|result| *result = committed_result(TRANSACTION_ID, &sets[3])).await;
        let (_client, pending) = pending(&indexer, IndexerTrust::Verify);

        let err = pending.watch().await.unwrap_err();
        assert!(
            matches!(
                err,
                PendingTransactionError::Unverified(VerifyError::ReceiptProvenBeforeAnchor {
                    epoch: Epoch(4),
                    anchor_epoch: Epoch(5),
                    ..
                })
            ),
            "{err}"
        );
    }

    #[tokio::test]
    async fn a_trusting_provider_reports_an_abort_whose_execution_accepted_as_rejected() {
        let indexer = indexer_that_finalized(|result| {
            let IndexerTransactionFinalizedResult::Finalized { final_decision, .. } = &mut result.result else {
                panic!("not a finalized result");
            };
            *final_decision = Decision::Abort(AbortReason::LockInputsFailed);
        })
        .await;
        let (_client, pending) = pending(&indexer, IndexerTrust::Trust);

        let err = pending.watch().await.unwrap_err();
        assert!(
            matches!(err, PendingTransactionError::TransactionRejected { .. }),
            "{err}"
        );
    }

    #[tokio::test]
    async fn the_event_stream_pauses_once_a_watch_is_answered_by_asking_the_indexer() {
        let indexer = indexer_that_finalized(|_| {}).await;
        let (_client, pending, mut pause_waiter) = watched(&indexer, IndexerTrust::Verify);
        let pending = pending.with_timeout(Duration::from_secs(60));

        pending.get_receipt().await.unwrap();
        tokio::time::timeout(Duration::from_secs(3), pause_waiter.wait_paused())
            .await
            .expect("the event stream stays unpaused for the answered watch");
    }
}
