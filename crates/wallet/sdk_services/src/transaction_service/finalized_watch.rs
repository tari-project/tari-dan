//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{pin::Pin, time::Duration};

use futures::StreamExt;
use log::*;
use tari_ootle_wallet_sdk::network::{
    TransactionFinalizedNotification,
    TransactionFinalizedStream,
    WalletNetworkInterface,
};
use tokio::{
    sync::mpsc,
    time::{Instant, Sleep, sleep_until},
};

const LOG_TARGET: &str = "tari::ootle::wallet_services::transaction_service::finalized_watch";

/// A subscription to the network's transaction finalization stream that reconnects after a fixed backoff whenever
/// the stream drops.
pub(super) struct FinalizedWatch<TNetwork: WalletNetworkInterface> {
    network: TNetwork,
    state: State<TNetwork::Error>,
    reconnect_backoff: Duration,
}

enum State<E> {
    Connected(TransactionFinalizedStream<E>),
    Reconnecting(Pin<Box<Sleep>>),
}

pub(super) enum WatchEvent {
    /// A subscription is established. Notifications for transactions finalized while disconnected were missed.
    Connected,
    Finalized(TransactionFinalizedNotification),
    Disconnected,
}

impl<TNetwork> FinalizedWatch<TNetwork>
where TNetwork: WalletNetworkInterface + Send + 'static
{
    /// Runs the watch on its own task, delivering events to the returned channel until the receiver is dropped.
    pub fn spawn(network: TNetwork, reconnect_backoff: Duration) -> mpsc::Receiver<WatchEvent> {
        let (tx, rx) = mpsc::channel(64);
        let mut watch = Self {
            network,
            state: State::Reconnecting(Box::pin(sleep_until(Instant::now()))),
            reconnect_backoff,
        };
        tokio::spawn(async move {
            loop {
                let event = watch.next_event().await;
                if tx.send(event).await.is_err() {
                    break;
                }
            }
        });
        rx
    }

    /// Resolves with the next watch event. Failed reconnect attempts are logged and retried after the backoff
    /// without resolving, so a caller only observes `Connected`, `Finalized` and `Disconnected` transitions.
    async fn next_event(&mut self) -> WatchEvent {
        loop {
            match &mut self.state {
                State::Connected(stream) => {
                    match stream.next().await {
                        Some(Ok(notification)) => return WatchEvent::Finalized(notification),
                        Some(Err(err)) => {
                            warn!(target: LOG_TARGET, "Transaction finalization stream failed: {err}");
                        },
                        None => {
                            warn!(target: LOG_TARGET, "Transaction finalization stream ended");
                        },
                    }
                    self.schedule_reconnect();
                    return WatchEvent::Disconnected;
                },
                State::Reconnecting(sleep) => {
                    sleep.as_mut().await;
                    match self.network.subscribe_transaction_finalized().await {
                        Ok(stream) => {
                            self.state = State::Connected(stream);
                            return WatchEvent::Connected;
                        },
                        Err(err) => {
                            warn!(
                                target: LOG_TARGET,
                                "Failed to subscribe to transaction finalization stream, retrying in {:?}: {err}",
                                self.reconnect_backoff
                            );
                            self.schedule_reconnect();
                        },
                    }
                },
            }
        }
    }

    fn schedule_reconnect(&mut self) {
        self.state = State::Reconnecting(Box::pin(sleep_until(Instant::now() + self.reconnect_backoff)));
    }
}
