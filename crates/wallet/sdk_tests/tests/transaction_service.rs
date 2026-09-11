//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

mod support;

use std::{
    collections::{HashMap, VecDeque},
    sync::{
        Arc,
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use futures::StreamExt;
use tari_consensus_types::Decision;
use tari_crypto::ristretto::RistrettoSecretKey;
use tari_engine_types::{
    Epoch,
    Utxo,
    commit_result::{AbortReason, ExecuteResult, FinalizeResult, TransactionResult},
    fees::FeeReceipt,
    substate::{Substate, SubstateDiff, SubstateId},
    transaction_receipt::FinalizeOutcome,
};
use tari_indexer_client::types::WatchedSubstateItem;
use tari_ootle_common_types::{
    StateVersion,
    optional::IsNotFoundError,
    response_status::{ResponseErrorStatus, TransactionStatusResponseError},
    shard::Shard,
};
use tari_ootle_transaction::{Transaction, TransactionEnvelope, TransactionId, args};
use tari_ootle_wallet_sdk::{
    models::{TransactionStatus, WalletEvent},
    network::{
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
#[derive(Debug, Clone)]
struct ScriptedNetwork {
    results: Arc<Mutex<VecDeque<TransactionFinalizedResult>>>,
    query_count: Arc<AtomicUsize>,
    notifications: Arc<Mutex<Option<mpsc::UnboundedReceiver<TransactionFinalizedNotification>>>>,
}

impl ScriptedNetwork {
    fn new(
        results: Vec<TransactionFinalizedResult>,
    ) -> (Self, mpsc::UnboundedSender<TransactionFinalizedNotification>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let network = Self {
            results: Arc::new(Mutex::new(results.into())),
            query_count: Arc::new(AtomicUsize::new(0)),
            notifications: Arc::new(Mutex::new(Some(rx))),
        };
        (network, tx)
    }

    fn query_count(&self) -> usize {
        self.query_count.load(Ordering::SeqCst)
    }
}

impl WalletNetworkInterface for ScriptedNetwork {
    type Error = ScriptedError;

    async fn query_substate(
        &self,
        _address: &SubstateId,
        _version: Option<u64>,
        _local_search_only: bool,
    ) -> Result<SubstateQueryResult, Self::Error> {
        panic!("ScriptedNetwork::query_substate called")
    }

    async fn get_substates(&self, _: Vec<SubstateId>) -> Result<HashMap<SubstateId, Substate>, Self::Error> {
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
    ) -> Result<TransactionQueryResult, Self::Error> {
        self.query_count.fetch_add(1, Ordering::SeqCst);
        let mut results = self.results.lock().unwrap();
        let result = if results.len() > 1 {
            results.pop_front().unwrap()
        } else {
            results.front().cloned().expect("no scripted result")
        };
        Ok(TransactionQueryResult { transaction_id, result })
    }

    async fn subscribe_transaction_finalized(&self) -> Result<TransactionFinalizedStream<Self::Error>, Self::Error> {
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
    Transaction::builder_localnet(Epoch(100))
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

/// Starts the service and waits for its startup subscription and the poll that follows it to settle.
async fn start(config: TransactionServiceConfig, network: ScriptedNetwork) -> Running {
    let test = TestWithNetwork::with_network(network);
    let notify = Notify::new(16);
    let shutdown = Shutdown::new();
    let (service, handle) =
        TransactionService::with_config(config, notify.clone(), test.sdk().clone(), shutdown.to_signal());
    let events = notify.subscribe();
    tokio::spawn(service.run());
    tokio::time::sleep(Duration::from_millis(100)).await;
    Running {
        handle,
        events,
        _shutdown: shutdown,
        _test: test,
    }
}

/// Waits for the wallet to report `transaction_id` finalized, returning its status.
async fn wait_for_finalized(
    events: &mut broadcast::Receiver<WalletEvent>,
    transaction_id: TransactionId,
    timeout: Duration,
) -> Option<TransactionStatus> {
    tokio::time::timeout(timeout, async {
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
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn finalization_notification_is_acted_on_without_waiting_for_the_poll() {
    let transaction = build_transaction();
    let transaction_id = transaction.calculate_id();
    let (network, notifications) =
        ScriptedNetwork::new(vec![TransactionFinalizedResult::Pending, committed(transaction_id)]);
    let mut running = start(fast_config(), network.clone()).await;

    running.handle.submit_transaction(transaction).await.unwrap();
    // The post-submit check finds it pending; nothing else queries a transaction younger than the silent timeout.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(network.query_count(), 1);

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

    let status = wait_for_finalized(&mut running.events, transaction_id, Duration::from_secs(5)).await;
    assert_eq!(status, Some(TransactionStatus::Accepted));
    assert_eq!(network.query_count(), 2);
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

    let status = wait_for_finalized(&mut running.events, transaction_id, Duration::from_secs(10)).await;
    assert_eq!(status, Some(TransactionStatus::Rejected));
    // One post-submit query, then one after the transaction had been silent for the timeout.
    assert_eq!(network.query_count(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn every_pending_transaction_is_polled_while_the_stream_is_down() {
    let transaction = build_transaction();
    let transaction_id = transaction.calculate_id();
    let (network, notifications) =
        ScriptedNetwork::new(vec![TransactionFinalizedResult::Pending, committed(transaction_id)]);
    let mut running = start(fast_config(), network.clone()).await;

    drop(notifications);
    // Wait for the service to observe the disconnect.
    tokio::time::sleep(Duration::from_millis(100)).await;

    running.handle.submit_transaction(transaction).await.unwrap();

    let status = wait_for_finalized(&mut running.events, transaction_id, Duration::from_secs(5)).await;
    assert_eq!(status, Some(TransactionStatus::Accepted));
}
