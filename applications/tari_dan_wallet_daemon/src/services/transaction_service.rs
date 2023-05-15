//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{sync::Arc, time::Duration};

use log::*;
use tari_dan_common_types::optional::IsNotFoundError;
use tari_dan_wallet_sdk::{
    apis::transaction::TransactionApiError,
    models::TransactionStatus,
    storage::WalletStore,
    substate_provider::WalletNetworkInterface,
    DanWalletSdk,
};
use tari_shutdown::ShutdownSignal;
use tokio::{
    sync::{watch, Semaphore},
    time,
    time::MissedTickBehavior,
};

use crate::{
    notify::Notify,
    services::{TransactionFinalizedEvent, TransactionInvalidEvent, WalletEvent},
};

const LOG_TARGET: &str = "tari::dan::wallet_daemon::transaction_service";

pub struct TransactionService<TStore, TNetworkInterface> {
    notify: Notify<WalletEvent>,
    wallet_sdk: DanWalletSdk<TStore, TNetworkInterface>,
    trigger_poll: watch::Sender<()>,
    rx_trigger: watch::Receiver<()>,
    poll_semaphore: Arc<Semaphore>,
    shutdown_signal: ShutdownSignal,
}

impl<TStore, TNetworkInterface> TransactionService<TStore, TNetworkInterface>
where
    TStore: WalletStore + Clone + Send + Sync + 'static,
    TNetworkInterface: WalletNetworkInterface + Clone + Send + Sync + 'static,
    TNetworkInterface::Error: IsNotFoundError,
{
    pub fn new(
        notify: Notify<WalletEvent>,
        wallet_sdk: DanWalletSdk<TStore, TNetworkInterface>,
        shutdown_signal: ShutdownSignal,
    ) -> Self {
        let (trigger, rx_trigger) = watch::channel(());
        Self {
            notify,
            wallet_sdk,
            trigger_poll: trigger,
            rx_trigger,
            poll_semaphore: Arc::new(Semaphore::new(1)),
            shutdown_signal,
        }
    }

    pub async fn run(mut self) -> Result<(), anyhow::Error> {
        let mut events_subscription = self.notify.subscribe();
        let mut poll_interval = time::interval(Duration::from_secs(10));
        poll_interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                _ = self.shutdown_signal.wait() => {
                    break Ok(());
                }
                Ok(event) = events_subscription.recv() => {
                    if let Err(e) = self.on_event(event) {
                        error!(target: LOG_TARGET, "Error handling event: {}", e);
                    }
                },

                Ok(_) = self.rx_trigger.changed() => {
                    trace!(target: LOG_TARGET, "Polling for transactions");
                    self.on_poll().await?;
                }

                _ = poll_interval.tick() => {
                    trace!(target: LOG_TARGET, "Polling for transactions");
                    self.on_poll().await?;
                }
            }
        }
    }

    async fn on_poll(&self) -> Result<(), TransactionServiceError> {
        let permit = match self.poll_semaphore.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                debug!(target: LOG_TARGET, "Polling is already in progress");
                return Ok(());
            },
        };

        let wallet_sdk = self.wallet_sdk.clone();
        let notify = self.notify.clone();
        tokio::spawn(async move {
            if let Err(err) = Self::check_pending_transactions(wallet_sdk, notify).await {
                error!(target: LOG_TARGET, "Error checking pending transactions: {}", err);
            }

            drop(permit);
        });
        Ok(())
    }

    async fn check_pending_transactions(
        wallet_sdk: DanWalletSdk<TStore, TNetworkInterface>,
        notify: Notify<WalletEvent>,
    ) -> Result<(), TransactionServiceError> {
        let transaction_api = wallet_sdk.transaction_api();
        let pending_transactions = transaction_api.fetch_all_by_status(TransactionStatus::Pending)?;
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
            info!(
                target: LOG_TARGET,
                "Requesting result for transaction {}",
                transaction.transaction.hash()
            );
            let maybe_finalized_transaction = transaction_api
                .check_and_store_finalized_transaction(transaction.transaction.hash().into_array().into())
                .await?;

            match maybe_finalized_transaction {
                Some(transaction) => {
                    debug!(
                        target: LOG_TARGET,
                        "Transaction {} has been finalized: {:?}",
                        transaction.transaction.hash(),
                        transaction.status,
                    );

                    match transaction.finalize {
                        Some(finalize) => {
                            notify.notify(TransactionFinalizedEvent {
                                hash: transaction.transaction.hash().into_array().into(),
                                finalize,
                                transaction_failure: transaction.transaction_failure,
                                final_fee: transaction.final_fee.unwrap_or_default(),
                                qcs: transaction.qcs,
                                status: transaction.status,
                            });
                        },
                        None => notify.notify(TransactionInvalidEvent {
                            hash: transaction.transaction.hash().into_array().into(),
                            status: transaction.status,
                            final_fee: transaction.final_fee.unwrap_or_default(),
                        }),
                    }
                },
                None => {
                    debug!(
                        target: LOG_TARGET,
                        "Transaction {} is still pending",
                        transaction.transaction.hash()
                    );
                },
            }
        }
        Ok(())
    }

    fn on_event(&mut self, event: WalletEvent) -> Result<(), TransactionServiceError> {
        match event {
            WalletEvent::TransactionSubmitted(_) => {
                let _ = self.trigger_poll.send(());
            },
            WalletEvent::TransactionInvalid(_) |
            WalletEvent::TransactionFinalized(_) |
            WalletEvent::AccountChanged(_) => {},
            WalletEvent::AuthLoginRequest(_) => {},
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TransactionServiceError {
    #[error("Transaction API error: {0}")]
    TransactionApiError(#[from] TransactionApiError),
}
