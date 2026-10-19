//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_consensus::hotstuff::{ConsensusCurrentState, CurrentView, HotstuffEvent};
use tari_ootle_common_types::Epoch;
use tari_ootle_transaction::Transaction;
use tokio::sync::{broadcast, mpsc, watch};

use crate::event_subscription::EventSubscription;

#[derive(Debug, Clone)]
pub struct ConsensusHandle {
    rx_current_state: watch::Receiver<ConsensusCurrentState>,
    events_subscription: EventSubscription<HotstuffEvent>,
    current_view: CurrentView,
    tx_new_transaction: mpsc::Sender<(Transaction, usize)>,
}

impl ConsensusHandle {
    pub(super) fn new(
        rx_current_state: watch::Receiver<ConsensusCurrentState>,
        events_subscription: EventSubscription<HotstuffEvent>,
        current_view: CurrentView,
        tx_new_transaction: mpsc::Sender<(Transaction, usize)>,
    ) -> Self {
        Self {
            rx_current_state,
            events_subscription,
            current_view,
            tx_new_transaction,
        }
    }

    /// A handle to a consensus that is not running, publishing its events on `events`.
    #[cfg(test)]
    pub(crate) fn for_test(events: &broadcast::Sender<HotstuffEvent>) -> Self {
        let (_, rx_current_state) = watch::channel(Default::default());
        let (tx_new_transaction, _) = mpsc::channel(1);
        Self::new(
            rx_current_state,
            EventSubscription::new(events.downgrade()),
            CurrentView::new(),
            tx_new_transaction,
        )
    }

    pub fn current_epoch(&self) -> Epoch {
        self.current_view.get_epoch()
    }

    pub async fn notify_new_transaction(
        &self,
        transaction: Transaction,
        num_pending: usize,
    ) -> Result<(), mpsc::error::SendError<()>> {
        self.tx_new_transaction
            .send((transaction, num_pending))
            .await
            .map_err(|_| mpsc::error::SendError(()))
    }

    pub fn current_view(&self) -> &CurrentView {
        &self.current_view
    }

    pub fn subscribe_to_hotstuff_events(&mut self) -> Result<broadcast::Receiver<HotstuffEvent>, anyhow::Error> {
        Ok(self.events_subscription.try_subscribe()?)
    }

    pub fn get_current_state(&self) -> ConsensusCurrentState {
        *self.rx_current_state.borrow()
    }

    pub fn is_running(&self) -> bool {
        self.get_current_state().is_running()
    }

    /// True while this node's committed history may be served to peers. A node that has left the
    /// register still holds what it committed while it was in one, and is the only source of it for
    /// the committee that took over, so `Idle` serves. While consensus is initialising or syncing,
    /// what this node has committed is not what its committee has committed, so it must not answer.
    pub fn can_serve_committed_state(&self) -> bool {
        matches!(
            self.get_current_state(),
            ConsensusCurrentState::Running | ConsensusCurrentState::Idle
        )
    }
}
