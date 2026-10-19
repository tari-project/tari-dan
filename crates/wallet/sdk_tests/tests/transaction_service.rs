//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

mod support;

use std::{
    collections::{HashSet, VecDeque},
    sync::{
        Arc,
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use futures::StreamExt;
use tari_consensus_types::Decision;
use tari_crypto::ristretto::RistrettoSecretKey;
use tari_engine_types::{
    Epoch,
    Utxo,
    commit_result::{AbortReason, ExecuteResult, FinalizeResult, TransactionResult},
    fees::FeeReceipt,
    substate::{SubstateDiff, SubstateId},
    transaction_receipt::FinalizeOutcome,
};
use tari_indexer_client::types::{EpochCheckpointEntry, ValidatorSetCommittee, WatchedSubstateItem};
use tari_ootle_common_types::{
    StateVersion,
    SubstateVersion,
    optional::IsNotFoundError,
    response_status::{ResponseErrorStatus, TransactionStatusResponseError},
    shard::Shard,
};
use tari_ootle_transaction::{Transaction, TransactionEnvelope, TransactionId, args};
use tari_ootle_wallet_sdk::{
    models::{TransactionStatus, WalletEvent},
    network::{
        FetchedSubstates,
        SubstateQueryResult,
        TransactionFinalizedNotification,
        TransactionFinalizedResult,
        TransactionFinalizedStream,
        TransactionQueryResult,
        UtxoUpdateStream,
        WalletNetworkInterface,
    },
    storage::TagAndPublicNoncePair,
};
use tari_ootle_wallet_sdk_services::{
    notify::Notify,
    transaction_service::{TransactionService, TransactionServiceConfig, TransactionServiceHandle},
};
use tari_shutdown::Shutdown;
use tari_template_abi::TemplateDef;
use tari_template_lib::types::{ResourceAddress, TemplateAddress, UtxoId};
use time::{OffsetDateTime, PrimitiveDateTime};
use tokio::sync::{broadcast, mpsc};

use crate::support::TestWithNetwork;

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
struct ScriptedError(&'static str);

impl IsNotFoundError for ScriptedError {
    fn is_not_found_error(&self) -> bool {
        false
    }
}

impl TransactionStatusResponseError for ScriptedError {
    fn get_status(&self) -> ResponseErrorStatus {
        ResponseErrorStatus::InternalError {
            message: self.0.to_string(),
        }
    }

    fn get_error_message(&self) -> String {
        self.0.to_string()
    }
}

/// A network that answers each result query with the next scripted result (the last one repeats) and lets the test
/// push finalization notifications onto the subscription stream. Only the first subscription succeeds: once its
/// sender is dropped the stream ends and every re-subscription fails, which is how a test takes the stream down.
///
/// A test that needs to know which query saw a result scripts a single repeating one and swaps it with
/// [`Self::set_result`] at the moment of its choosing, rather than relying on queries arriving in a fixed order.
#[derive(Debug, Clone)]
struct ScriptedNetwork {
    results: Arc<Mutex<VecDeque<TransactionFinalizedResult>>>,
    query_count: Arc<AtomicUsize>,
    subscribe_count: Arc<AtomicUsize>,
    notifications: Arc<Mutex<Option<mpsc::UnboundedReceiver<TransactionFinalizedNotification>>>>,
    /// Transactions whose result query fails.
    failing: Arc<Mutex<HashSet<TransactionId>>>,
}

impl ScriptedNetwork {
    fn new(
        results: Vec<TransactionFinalizedResult>,
    ) -> (Self, mpsc::UnboundedSender<TransactionFinalizedNotification>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let network = Self {
            results: Arc::new(Mutex::new(results.into())),
            query_count: Arc::new(AtomicUsize::new(0)),
            subscribe_count: Arc::new(AtomicUsize::new(0)),
            notifications: Arc::new(Mutex::new(Some(rx))),
            failing: Arc::new(Mutex::new(HashSet::new())),
        };
        (network, tx)
    }

    fn query_count(&self) -> usize {
        self.query_count.load(Ordering::SeqCst)
    }

    /// Subscription attempts, refused ones included. The first tells a test the service is listening; a second
    /// tells it the service has noticed the stream end and fallen back to polling.
    fn subscribe_count(&self) -> usize {
        self.subscribe_count.load(Ordering::SeqCst)
    }

    /// Makes every result query for `transaction_id` fail from now on.
    fn fail_queries_for(&self, transaction_id: TransactionId) {
        self.failing.lock().unwrap().insert(transaction_id);
    }

    /// Makes `result` the answer to every query from now on.
    fn set_result(&self, result: TransactionFinalizedResult) {
        let mut results = self.results.lock().unwrap();
        results.clear();
        results.push_back(result);
    }
}

impl WalletNetworkInterface for ScriptedNetwork {
    type Error = ScriptedError;

    async fn query_substate(
        &self,
        _address: &SubstateId,
        _version: Option<SubstateVersion>,
        _local_search_only: bool,
        _include_proof: bool,
    ) -> Result<SubstateQueryResult, Self::Error> {
        panic!("ScriptedNetwork::query_substate called")
    }

    async fn get_substates(&self, _: Vec<SubstateId>, _: bool) -> Result<FetchedSubstates, Self::Error> {
        panic!("ScriptedNetwork::get_substates called")
    }

    async fn submit_transaction(&self, transaction: Transaction) -> Result<TransactionId, Self::Error> {
        Ok(transaction.calculate_id())
    }

    async fn submit_transaction_envelope(&self, _: TransactionEnvelope) -> Result<TransactionId, Self::Error> {
        panic!("ScriptedNetwork::submit_transaction_envelope called")
    }

    async fn submit_dry_run_transaction(&self, _: Transaction) -> Result<TransactionQueryResult, Self::Error> {
        panic!("ScriptedNetwork::submit_dry_run_transaction called")
    }

    async fn query_transaction_result(
        &self,
        transaction_id: TransactionId,
        _include_proof: bool,
    ) -> Result<TransactionQueryResult, Self::Error> {
        self.query_count.fetch_add(1, Ordering::SeqCst);
        if self.failing.lock().unwrap().contains(&transaction_id) {
            return Err(ScriptedError("the result query failed"));
        }
        let mut results = self.results.lock().unwrap();
        let result = if results.len() > 1 {
            results.pop_front().unwrap()
        } else {
            results.front().cloned().expect("no scripted result")
        };
        Ok(TransactionQueryResult {
            transaction_id,
            result,
            receipt: None,
        })
    }

    async fn subscribe_transaction_finalized(&self) -> Result<TransactionFinalizedStream<Self::Error>, Self::Error> {
        self.subscribe_count.fetch_add(1, Ordering::SeqCst);
        match self.notifications.lock().unwrap().take() {
            Some(rx) => Ok(tokio_stream::wrappers::UnboundedReceiverStream::new(rx).map(Ok).boxed()),
            None => Err(ScriptedError("finalization stream unavailable")),
        }
    }

    async fn fetch_template_definition(&self, _: TemplateAddress) -> Result<TemplateDef, Self::Error> {
        panic!("ScriptedNetwork::fetch_template_definition called")
    }

    async fn stream_stealth_utxo_updates(
        &self,
        _: Epoch,
        _: ResourceAddress,
        _: Vec<(Shard, StateVersion)>,
        _: bool,
    ) -> Result<UtxoUpdateStream<Self::Error>, Self::Error> {
        panic!("ScriptedNetwork::stream_stealth_utxo_updates called")
    }

    async fn list_watched_substates(
        &self,
        _: Option<TemplateAddress>,
        _: Option<u64>,
        _: Option<u64>,
    ) -> Result<Vec<WatchedSubstateItem>, Self::Error> {
        panic!("ScriptedNetwork::list_watched_substates called")
    }

    async fn get_unspent_utxos(
        &self,
        _: ResourceAddress,
        _: Vec<TagAndPublicNoncePair>,
    ) -> Result<Vec<(UtxoId, Utxo)>, Self::Error> {
        panic!("ScriptedNetwork::get_unspent_utxos called")
    }

    async fn get_latest_epoch_checkpoint(&self, _shard: Shard) -> Result<EpochCheckpointEntry, Self::Error> {
        panic!("ScriptedNetwork called")
    }

    async fn list_epoch_checkpoints(
        &self,
        _from_epoch: Epoch,
        _limit: u32,
        _shard: Shard,
    ) -> Result<Vec<EpochCheckpointEntry>, Self::Error> {
        panic!("ScriptedNetwork called")
    }

    async fn get_committee(&self, _epoch: Epoch, _shard: Shard) -> Result<ValidatorSetCommittee, Self::Error> {
        panic!("ScriptedNetwork called")
    }

    async fn get_current_epoch(&self) -> Result<Epoch, Self::Error> {
        panic!("ScriptedNetwork::get_current_epoch called")
    }

    async fn wait_until_ready(&self) -> Result<(), Self::Error> {
        Ok(())
    }
}

fn now() -> PrimitiveDateTime {
    let now = OffsetDateTime::now_utc();
    PrimitiveDateTime::new(now.date(), now.time())
}

fn build_transaction() -> Transaction {
    build_transaction_valid_to(Epoch(100))
}

fn build_transaction_valid_to(max_epoch: Epoch) -> Transaction {
    Transaction::builder_localnet(max_epoch)
        .allocate_component_address("component")
        .put_last_instruction_output_on_workspace("bucket")
        .call_method("component", "new", args!["bucket"])
        .build_and_seal(&RistrettoSecretKey::from(1))
}

fn committed(transaction_id: TransactionId) -> TransactionFinalizedResult {
    let finalize = FinalizeResult::new(
        transaction_id.into_array().into(),
        vec![],
        vec![],
        TransactionResult::Accept(SubstateDiff::new()),
        FeeReceipt::default(),
    );
    TransactionFinalizedResult::Finalized {
        final_decision: Decision::Commit,
        execution_result: Some(Box::new(ExecuteResult {
            finalize,
            execution_time: Duration::from_secs(1),
            execute_epoch: None,
            wasm_execution_points: 0,
            native_execution_points: 0,
        })),
        execution_time: Duration::from_secs(1),
        finalized_time: now(),
        abort_details: None,
    }
}

fn aborted() -> TransactionFinalizedResult {
    TransactionFinalizedResult::Finalized {
        final_decision: Decision::Abort(AbortReason::LockInputsFailed),
        execution_result: None,
        execution_time: Duration::ZERO,
        finalized_time: now(),
        abort_details: Some("inputs locked".to_string()),
    }
}

struct Running {
    handle: TransactionServiceHandle,
    events: broadcast::Receiver<WalletEvent>,
    _shutdown: Shutdown,
    _test: TestWithNetwork<ScriptedNetwork>,
}

/// Starts the service and waits for it to subscribe to the finalization stream, so that a test that pushes a
/// notification knows there is something listening for it.
async fn start(config: TransactionServiceConfig, network: ScriptedNetwork) -> Running {
    let watched = network.clone();
    let test = TestWithNetwork::with_network(network);
    let notify = Notify::new(16);
    let shutdown = Shutdown::new();
    let (service, handle) =
        TransactionService::with_config(config, notify.clone(), test.sdk().clone(), shutdown.to_signal());
    let events = notify.subscribe();
    tokio::spawn(service.run());
    assert!(
        wait_until(|| watched.subscribe_count() >= 1).await,
        "service never subscribed to the finalization stream"
    );
    Running {
        handle,
        events,
        _shutdown: shutdown,
        _test: test,
    }
}

/// How long a test waits for the service to do something before calling it stuck. This is a deadlock detector,
/// not a performance assertion: the suite runs its tests in parallel, each on its own multi-threaded runtime, so a
/// loaded machine can starve any one of them for seconds at a time. It sits far above the work it covers, because
/// a deadline that merely exceeds the expected duration fails the run instead of the code.
const WAIT_TIMEOUT: Duration = Duration::from_secs(30);

/// How long a test watches for something that must *not* happen. Starvation can only cut such a window short,
/// weakening the check, never failing it, so this stays small.
const QUIET_WINDOW: Duration = Duration::from_millis(300);

/// Waits until the network stops being queried and returns the count it settled at. The subscription catch-up
/// poll and the post-submit check race each other, so the number of queries a test has seen by the time it is
/// ready to act is not fixed; what is fixed is that the count stops moving once both have run.
async fn wait_until_queries_settle(network: &ScriptedNetwork) -> usize {
    let deadline = Instant::now() + WAIT_TIMEOUT;
    loop {
        let before = network.query_count();
        tokio::time::sleep(QUIET_WINDOW).await;
        if network.query_count() == before {
            return before;
        }
        assert!(Instant::now() < deadline, "the network never stopped being queried");
    }
}

/// Polls until `condition` holds. Returns false if it has not held within [`WAIT_TIMEOUT`].
async fn wait_until(mut condition: impl FnMut() -> bool) -> bool {
    tokio::time::timeout(WAIT_TIMEOUT, async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .is_ok()
}

/// Waits for the wallet to report `transaction_id` finalized, returning its status.
async fn wait_for_finalized(
    events: &mut broadcast::Receiver<WalletEvent>,
    transaction_id: TransactionId,
) -> Option<TransactionStatus> {
    tokio::time::timeout(WAIT_TIMEOUT, async {
        loop {
            match events.recv().await.unwrap() {
                WalletEvent::TransactionFinalized(event) if event.transaction_id == transaction_id => {
                    break event.status;
                },
                WalletEvent::TransactionInvalid(event) if event.transaction_id == transaction_id => {
                    break event.status;
                },
                _ => {},
            }
        }
    })
    .await
    .ok()
}

fn fast_config() -> TransactionServiceConfig {
    TransactionServiceConfig {
        poll_interval: Duration::from_millis(50),
        post_submit_check_delay: Duration::from_millis(20),
        silent_transaction_timeout: Duration::from_secs(60),
        stream_reconnect_backoff: Duration::from_millis(20),
        validator_set_sync_interval: Duration::from_secs(60 * 60),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn finalization_notification_is_acted_on_without_waiting_for_the_poll() {
    let transaction = build_transaction();
    let transaction_id = transaction.calculate_id();
    let (network, notifications) = ScriptedNetwork::new(vec![TransactionFinalizedResult::Pending]);
    let mut running = start(fast_config(), network.clone()).await;

    running.handle.submit_transaction(transaction).await.unwrap();
    // The post-submit check finds it pending, and nothing then queries a transaction younger than the silent
    // timeout, so the count settles. Every query so far has seen `Pending`, so none of them can have finalized it.
    let settled = wait_until_queries_settle(&network).await;
    assert!(settled >= 1, "post-submit check never ran");

    // Only a query made from here on can see the commit, which is what makes the next one attributable.
    network.set_result(committed(transaction_id));

    // A notification for someone else's transaction is ignored.
    notifications
        .send(TransactionFinalizedNotification {
            transaction_id: TransactionId::new([9u8; 32]),
            outcome: FinalizeOutcome::Commit,
        })
        .unwrap();
    notifications
        .send(TransactionFinalizedNotification {
            transaction_id,
            outcome: FinalizeOutcome::Commit,
        })
        .unwrap();

    let status = wait_for_finalized(&mut running.events, transaction_id).await;
    assert_eq!(status, Some(TransactionStatus::Accepted));
    // One further query, and the notification is the only thing that could have caused it: the poll skips a
    // transaction younger than the silent timeout.
    assert_eq!(network.query_count(), settled + 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn abort_is_reported_without_a_notification() {
    let transaction = build_transaction();
    let transaction_id = transaction.calculate_id();
    let (network, _notifications) = ScriptedNetwork::new(vec![TransactionFinalizedResult::Pending, aborted()]);
    let config = TransactionServiceConfig {
        silent_transaction_timeout: Duration::from_secs(1),
        ..fast_config()
    };
    let mut running = start(config, network.clone()).await;

    running.handle.submit_transaction(transaction).await.unwrap();

    let status = wait_for_finalized(&mut running.events, transaction_id).await;
    assert_eq!(status, Some(TransactionStatus::Rejected));
    // The post-submit check found it pending and a later poll, once it had been silent for the timeout, found the
    // abort. How many queries the subscription catch-up adds alongside those is not fixed.
    assert!(network.query_count() >= 2, "the abort was reported without a poll");
}

#[tokio::test(flavor = "multi_thread")]
async fn every_pending_transaction_is_polled_while_the_stream_is_down() {
    let transaction = build_transaction();
    let transaction_id = transaction.calculate_id();
    let (network, notifications) =
        ScriptedNetwork::new(vec![TransactionFinalizedResult::Pending, committed(transaction_id)]);
    let mut running = start(fast_config(), network.clone()).await;

    drop(notifications);
    // The stream ends and the service re-subscribes. This network refuses every subscription after the first, so
    // the attempt marks the point from which the service has only the poll.
    assert!(
        wait_until(|| network.subscribe_count() >= 2).await,
        "service never re-subscribed after the stream ended"
    );

    running.handle.submit_transaction(transaction).await.unwrap();

    let status = wait_for_finalized(&mut running.events, transaction_id).await;
    assert_eq!(status, Some(TransactionStatus::Accepted));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_transaction_that_fails_to_check_does_not_hold_up_the_others() {
    let transaction = build_transaction();
    let transaction_id = transaction.calculate_id();
    let failing = build_transaction_valid_to(Epoch(101));
    let (network, notifications) = ScriptedNetwork::new(vec![TransactionFinalizedResult::Pending]);
    network.fail_queries_for(failing.calculate_id());
    let mut running = start(fast_config(), network.clone()).await;
    drop(notifications);
    assert!(
        wait_until(|| network.subscribe_count() >= 2).await,
        "service never re-subscribed after the stream ended"
    );

    running.handle.submit_transaction(transaction).await.unwrap();
    // The poll checks the newest pending transaction first, so the failing one goes ahead of the other.
    tokio::time::sleep(Duration::from_millis(1100)).await;
    running.handle.submit_transaction(failing).await.unwrap();
    network.set_result(committed(transaction_id));

    let status = wait_for_finalized(&mut running.events, transaction_id).await;
    assert_eq!(status, Some(TransactionStatus::Accepted));
}
