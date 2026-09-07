//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashSet, sync::Arc, time::Duration};

use log::*;
use tari_engine_types::commit_result::ExecuteResult;
use tari_ootle_common_types::{
    optional::{IsNotFoundError, Optional},
    response_status::TransactionStatusResponseError,
};
use tari_ootle_transaction::{Transaction, TransactionId};
use tari_ootle_wallet_sdk::{
    WalletSdk,
    WalletSdkSpec,
    models::{
        TransactionContext,
        TransactionContextKind,
        TransactionFinalizedEvent,
        TransactionInvalidEvent,
        TransactionStatus,
        TransactionSubmittedEvent,
        WalletEvent,
        WalletLockId,
        WalletTransaction,
    },
    network::{TransactionFinalizedNotification, WalletNetworkInterface},
};
use tari_shutdown::ShutdownSignal;
use tokio::{
    sync::{Semaphore, mpsc, watch},
    time,
    time::MissedTickBehavior,
};

use super::{
    error::TransactionServiceError,
    finalized_watch::{FinalizedWatch, WatchEvent},
    handle::{TransactionServiceHandle, TransactionServiceRequest},
};
use crate::notify::Notify;

const LOG_TARGET: &str = "tari::ootle::wallet_services::transaction_service";

#[derive(Debug, Clone, Copy)]
pub struct TransactionServiceConfig {
    /// Interval of the backstop poll that resubmits new transactions, clears stale locks and queries the result
    /// of transactions the finalization stream has stayed silent about.
    pub poll_interval: Duration,
    /// Delay between a submission and the first direct result query, which beats the finalization stream when the
    /// transaction commits within a block or so of submission.
    pub post_submit_check_delay: Duration,
    /// How long a pending transaction may go without a finalization notification before its result is queried
    /// directly. The stream never notifies about a transaction that aborts, so this bounds how late an abort is
    /// noticed; every poll after it costs one query per still-pending transaction.
    pub silent_transaction_timeout: Duration,
    /// Delay before re-subscribing to the finalization stream after it drops. While disconnected every poll
    /// queries every pending transaction.
    pub stream_reconnect_backoff: Duration,
}

impl Default for TransactionServiceConfig {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_secs(5),
            post_submit_check_delay: Duration::from_millis(750),
            silent_transaction_timeout: Duration::from_secs(10),
            stream_reconnect_backoff: Duration::from_secs(5),
        }
    }
}

pub struct TransactionService<TSpec: WalletSdkSpec> {
    rx_request: mpsc::Receiver<TransactionServiceRequest>,
    notify: Notify<WalletEvent>,
    wallet_sdk: WalletSdk<TSpec>,
    trigger_poll: watch::Sender<()>,
    rx_trigger: watch::Receiver<()>,
    poll_semaphore: Arc<Semaphore>,
    stream_connected: bool,
    /// Transactions this wallet is waiting on, so that the network-wide finalization stream can be filtered without
    /// touching the store. Seeded from the store on start and maintained from wallet events; a missed event is
    /// covered by the backstop poll.
    pending: HashSet<TransactionId>,
    /// Set when the next poll must query every pending transaction; cleared only once such a poll has run.
    check_all_on_next_tick: bool,
    config: TransactionServiceConfig,
    shutdown_signal: ShutdownSignal,
}

/// Which pending transactions a poll queries the network about.
#[derive(Debug, Clone, Copy)]
enum CheckScope {
    All,
    /// Only transactions that have been pending at least this long. Younger ones are expected to be reported by the
    /// finalization stream.
    PendingFor(Duration),
}

impl CheckScope {
    fn includes(&self, transaction: &WalletTransaction) -> bool {
        match self {
            CheckScope::All => true,
            CheckScope::PendingFor(min_age) => time_since(transaction.last_update_time) >= *min_age,
        }
    }
}

fn time_since(timestamp: ::time::PrimitiveDateTime) -> Duration {
    (::time::OffsetDateTime::now_utc() - timestamp.assume_utc())
        .try_into()
        .unwrap_or(Duration::ZERO)
}

impl<TSpec> TransactionService<TSpec>
where
    TSpec: WalletSdkSpec + Send + 'static,
    TSpec::Store: Clone + Send + Sync + 'static,
    TSpec::NetworkInterface: Clone + Send + Sync + 'static,
    TSpec::KeyStore: Clone + Send + Sync + 'static,
    <TSpec::NetworkInterface as WalletNetworkInterface>::Error: IsNotFoundError + TransactionStatusResponseError,
{
    pub fn new(
        notify: Notify<WalletEvent>,
        wallet_sdk: WalletSdk<TSpec>,
        shutdown_signal: ShutdownSignal,
    ) -> (Self, TransactionServiceHandle) {
        Self::with_config(TransactionServiceConfig::default(), notify, wallet_sdk, shutdown_signal)
    }

    pub fn with_config(
        config: TransactionServiceConfig,
        notify: Notify<WalletEvent>,
        wallet_sdk: WalletSdk<TSpec>,
        shutdown_signal: ShutdownSignal,
    ) -> (Self, TransactionServiceHandle) {
        let (trigger, rx_trigger) = watch::channel(());
        let (tx_request, rx_request) = mpsc::channel(1);
        let actor = Self {
            rx_request,
            notify,
            wallet_sdk,
            trigger_poll: trigger,
            rx_trigger,
            poll_semaphore: Arc::new(Semaphore::new(1)),
            stream_connected: false,
            pending: HashSet::new(),
            check_all_on_next_tick: false,
            config,
            shutdown_signal,
        };

        (actor, TransactionServiceHandle::new(tx_request))
    }

    pub async fn run(mut self) -> Result<(), anyhow::Error> {
        let mut events_subscription = self.notify.subscribe();
        let mut poll_interval = time::interval(self.config.poll_interval);
        poll_interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut rx_watch_events = FinalizedWatch::spawn(
            self.wallet_sdk.get_network_interface().clone(),
            self.config.stream_reconnect_backoff,
            self.shutdown_signal.clone(),
        );
        self.pending = self
            .wallet_sdk
            .transaction_api()
            .fetch_all(Some(TransactionStatus::Pending), None)?
            .into_iter()
            .map(|t| t.id)
            .collect();

        loop {
            tokio::select! {
                _ = self.shutdown_signal.wait() => {
                    break Ok(());
                }
                Some(req) = self.rx_request.recv() => {
                    if let Err(err) = self.handle_request(req).await {
                        error!(target: LOG_TARGET, "Error handling request: {}", err);
                    }
                },
                Ok(event) = events_subscription.recv() => {
                    if let Err(e) = self.on_event(event) {
                        error!(target: LOG_TARGET, "Error handling event: {}", e);
                    }
                },

                Ok(_) = self.rx_trigger.changed() => {
                    // Querying immediately after submission almost always finds the transaction still pending, so
                    // the first check waits long enough for a fast commit to be visible.
                    self.check_all_on_next_tick = true;
                    poll_interval.reset_after(self.config.post_submit_check_delay);
                }

                Some(watch_event) = rx_watch_events.recv() => {
                    match watch_event {
                        WatchEvent::Connected => {
                            info!(target: LOG_TARGET, "Subscribed to transaction finalization stream");
                            self.stream_connected = true;
                            // Anything finalized before the subscription was established was never notified.
                            if !self.on_poll(CheckScope::All)? {
                                self.check_all_on_next_tick = true;
                            }
                        },
                        WatchEvent::Finalized(notification) => {
                            if self.pending.contains(&notification.transaction_id) {
                                self.on_transaction_finalized(notification);
                            } else {
                                trace!(
                                    target: LOG_TARGET,
                                    "Ignoring finalization of transaction {} not pending in this wallet",
                                    notification.transaction_id
                                );
                            }
                        },
                        WatchEvent::Disconnected => {
                            self.stream_connected = false;
                            warn!(
                                target: LOG_TARGET,
                                "Transaction finalization stream disconnected. Polling every {:?} until it reconnects",
                                self.config.poll_interval
                            );
                        },
                    }
                }

                _ = poll_interval.tick() => {
                    let scope = if self.check_all_on_next_tick || !self.stream_connected {
                        CheckScope::All
                    } else {
                        CheckScope::PendingFor(self.config.silent_transaction_timeout)
                    };
                    trace!(target: LOG_TARGET, "Polling for transactions ({scope:?})");
                    if self.on_poll(scope)? {
                        self.check_all_on_next_tick = false;
                    }
                }
            }
        }
    }

    async fn handle_request(&self, request: TransactionServiceRequest) -> Result<(), TransactionServiceError> {
        match request {
            TransactionServiceRequest::SubmitTransaction {
                transaction,
                context,
                lock_id,
                reply,
            } => {
                reply
                    .send(self.handle_submit_transaction(transaction, context, lock_id).await)
                    .map_err(|_| TransactionServiceError::ServiceShutdown)?;
            },
            TransactionServiceRequest::SubmitDryRunTransaction {
                transaction,
                settled_fee,
                reply,
            } => {
                let transaction_id = transaction.calculate_id();
                let transaction_api = self.wallet_sdk.transaction_api();
                // Unlock all locks related to the transaction immediately since this is a dry run
                transaction_api.release_all_locks_for_transaction(transaction_id)?;
                match transaction_api
                    .submit_dry_run_transaction(transaction, settled_fee)
                    .await
                {
                    Ok(finalized_transaction) => {
                        let finalize =
                            finalized_transaction
                                .finalize
                                .ok_or_else(|| TransactionServiceError::InvariantError {
                                    details: format!(
                                        "Dry run transaction {transaction_id} succeeded but was not finalized"
                                    ),
                                });
                        reply
                            .send(finalize.map(|finalize| ExecuteResult {
                                finalize,
                                execution_time: finalized_transaction.execution_time.unwrap_or_default(),
                                execute_epoch: None,
                                wasm_execution_points: 0,
                                native_execution_points: 0,
                            }))
                            .map_err(|_| TransactionServiceError::ServiceShutdown)?;
                    },
                    Err(e) => {
                        reply
                            .send(Err(e.into()))
                            .map_err(|_| TransactionServiceError::ServiceShutdown)?;
                    },
                }
            },
        }
        Ok(())
    }

    async fn handle_submit_transaction(
        &self,
        transaction: Transaction,
        context: Option<TransactionContext>,
        lock_id: Option<WalletLockId>,
    ) -> Result<TransactionId, TransactionServiceError> {
        let transaction_api = self.wallet_sdk.transaction_api();
        let new_account_info = context.as_ref().and_then(|c| c.new_account_data()).cloned();
        let linked_accounts = context
            .as_ref()
            .map(|c| c.linked_accounts.as_slice())
            .unwrap_or_default();
        let transaction_id =
            transaction_api.insert_new_transaction(transaction, new_account_info, linked_accounts, false)?;

        if let Some(lock_id) = lock_id {
            transaction_api.locks_set_transaction_id(lock_id, transaction_id)?;
        }

        if transaction_api.submit_transaction(transaction_id).await? {
            self.notify.notify(TransactionSubmittedEvent {
                transaction_id,
                context,
            });
            Ok(transaction_id)
        } else {
            self.notify.notify(TransactionInvalidEvent {
                transaction_id,
                status: TransactionStatus::InvalidTransaction,
                finalize: None,
                final_fee: None,
            });
            Ok(transaction_id)
        }
    }

    /// Starts a poll, returning `false` without polling if one is already in progress.
    fn on_poll(&self, scope: CheckScope) -> Result<bool, TransactionServiceError> {
        let permit = match self.poll_semaphore.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                debug!(target: LOG_TARGET, "Polling is already in progress");
                return Ok(false);
            },
        };

        let wallet_sdk = self.wallet_sdk.clone();
        let notify = self.notify.clone();
        tokio::spawn(async move {
            if let Err(err) = Self::resubmit_new_transactions(&wallet_sdk, &notify).await {
                error!(target: LOG_TARGET, "Error resubmitting new transactions: {}", err);
            }
            if let Err(err) = Self::check_pending_transactions(&wallet_sdk, &notify, scope).await {
                error!(target: LOG_TARGET, "Error checking pending transactions: {}", err);
            }
            if let Err(err) = Self::clear_stale_locks(&wallet_sdk) {
                error!(target: LOG_TARGET, "Error clearing stale locks: {}", err);
            }

            drop(permit);
        });
        Ok(true)
    }

    /// Queries the result of a transaction the network reports as finalized and this wallet is waiting on.
    ///
    /// The check runs under the poll permit and re-reads the transaction's status once it holds it, so a poll that
    /// resolved the transaction in the meantime is not reported a second time.
    fn on_transaction_finalized(&self, notification: TransactionFinalizedNotification) {
        let tx_id = notification.transaction_id;
        let wallet_sdk = self.wallet_sdk.clone();
        let notify = self.notify.clone();
        let semaphore = self.poll_semaphore.clone();
        tokio::spawn(async move {
            let Ok(_permit) = semaphore.acquire_owned().await else {
                return;
            };
            if !Self::is_pending_in_wallet(&wallet_sdk, tx_id) {
                debug!(target: LOG_TARGET, "Transaction {tx_id} was resolved by a poll");
                return;
            }
            info!(
                target: LOG_TARGET,
                "Transaction {tx_id} finalized with outcome {:?}. Requesting result", notification.outcome
            );
            if let Err(err) = Self::check_pending_transaction(&wallet_sdk, &notify, tx_id).await {
                error!(target: LOG_TARGET, "Error checking finalized transaction {tx_id}: {err}");
            }
        });
    }

    fn is_pending_in_wallet(wallet_sdk: &WalletSdk<TSpec>, tx_id: TransactionId) -> bool {
        match wallet_sdk.transaction_api().get(tx_id).optional() {
            Ok(Some(transaction)) => transaction.status == TransactionStatus::Pending,
            Ok(None) => false,
            Err(err) => {
                error!(target: LOG_TARGET, "Error loading transaction {tx_id}: {err}");
                false
            },
        }
    }

    fn clear_stale_locks(wallet_sdk: &WalletSdk<TSpec>) -> Result<(), TransactionServiceError> {
        let transaction_api = wallet_sdk.locks_api();
        let num_cleared = transaction_api.clear_stale_locks()?;
        if num_cleared > 0 {
            info!(
                target: LOG_TARGET,
                "Cleared {} stale wallet lock(s)",
                num_cleared
            );
        } else {
            debug!(
                target: LOG_TARGET,
                "No stale wallet locks to clear",
            );
        }
        Ok(())
    }

    async fn resubmit_new_transactions(
        wallet_sdk: &WalletSdk<TSpec>,
        notify: &Notify<WalletEvent>,
    ) -> Result<(), TransactionServiceError> {
        let transaction_api = wallet_sdk.transaction_api();
        let new_transactions = transaction_api.fetch_all(Some(TransactionStatus::New), None)?;
        let log_level = if new_transactions.is_empty() {
            Level::Debug
        } else {
            Level::Info
        };
        log!(
            target: LOG_TARGET,
            log_level,
            "{} new transaction(s)",
            new_transactions.len()
        );
        for transaction in new_transactions {
            info!(
                target: LOG_TARGET,
                "Resubmitting transaction {}",
                transaction.id,
            );
            let transaction_id = transaction.id;
            if transaction_api.submit_transaction(transaction_id).await? {
                // Only NewAccountData is persisted in the DB, so only that context is recoverable on
                // resubmit. Other context variants (e.g. ClaimBurn) are in-memory only — if the daemon
                // restarts, that context is lost and the caller must retry.
                notify.notify(TransactionSubmittedEvent {
                    transaction_id,
                    context: transaction
                        .new_account_info
                        .map(|data| TransactionContext::default().with_kind(TransactionContextKind::NewAccount(data))),
                });
            } else {
                notify.notify(TransactionInvalidEvent {
                    transaction_id,
                    status: TransactionStatus::InvalidTransaction,
                    finalize: None,
                    final_fee: None,
                });
            }
        }
        Ok(())
    }

    async fn check_pending_transactions(
        wallet_sdk: &WalletSdk<TSpec>,
        notify: &Notify<WalletEvent>,
        scope: CheckScope,
    ) -> Result<(), TransactionServiceError> {
        let transaction_api = wallet_sdk.transaction_api();
        let pending_transactions = transaction_api.fetch_all(Some(TransactionStatus::Pending), None)?;
        let log_level = if pending_transactions.is_empty() {
            Level::Debug
        } else {
            Level::Info
        };
        log!(
            target: LOG_TARGET,
            log_level,
            "{} pending transaction(s)",
            pending_transactions.len()
        );
        for transaction in pending_transactions {
            let tx_id = transaction.id;
            if !scope.includes(&transaction) {
                trace!(
                    target: LOG_TARGET,
                    "Transaction {tx_id} pending for {:?}, waiting for the finalization stream",
                    time_since(transaction.last_update_time)
                );
                continue;
            }
            info!(
                target: LOG_TARGET,
                "Requesting result for transaction {tx_id}",
            );
            Self::check_pending_transaction(wallet_sdk, notify, tx_id).await?;
        }
        Ok(())
    }

    /// Queries the network for the result of a pending transaction, storing it and emitting the wallet event if it
    /// has finalized.
    async fn check_pending_transaction(
        wallet_sdk: &WalletSdk<TSpec>,
        notify: &Notify<WalletEvent>,
        tx_id: TransactionId,
    ) -> Result<(), TransactionServiceError> {
        let transaction_api = wallet_sdk.transaction_api();
        let maybe_finalized_transaction = transaction_api.check_and_store_finalized_transaction(tx_id).await?;

        match maybe_finalized_transaction {
            Some(transaction) => {
                debug!(
                    target: LOG_TARGET,
                    "Transaction {} has been finalized: {}",
                    transaction.id,
                    transaction.status,
                );
                match transaction.finalize {
                    Some(finalize) => {
                        notify.notify(TransactionFinalizedEvent {
                            transaction_id: tx_id,
                            finalize,
                            final_fee: transaction.final_fee.unwrap_or_default(),
                            status: transaction.status,
                        });
                    },
                    None => notify.notify(TransactionInvalidEvent {
                        transaction_id: tx_id,
                        status: transaction.status,
                        finalize: transaction.finalize,
                        final_fee: transaction.final_fee,
                    }),
                }
            },
            None => {
                debug!(
                    target: LOG_TARGET,
                    "Transaction {tx_id} is still pending",
                );
            },
        }
        Ok(())
    }

    fn on_event(&mut self, event: WalletEvent) -> Result<(), TransactionServiceError> {
        match event {
            WalletEvent::TransactionSubmitted(event) => {
                self.pending.insert(event.transaction_id);
                let _ = self.trigger_poll.send(());
            },
            WalletEvent::TransactionInvalid(event) => {
                self.pending.remove(&event.transaction_id);
            },
            WalletEvent::TransactionFinalized(event) => {
                self.pending.remove(&event.transaction_id);
            },
            WalletEvent::AccountChangedOnChain(_) |
            WalletEvent::AuthLoginRequest(_) |
            WalletEvent::AccountCreatedOnChain(_) |
            WalletEvent::UtxoRecoveryStarted(_) |
            WalletEvent::UtxoRecovered(_) |
            WalletEvent::UtxoRecoveryCompleted(_) |
            WalletEvent::UtxoSpent(_) |
            WalletEvent::TransactionRequestCreated(_) => {},
        }
        Ok(())
    }
}
