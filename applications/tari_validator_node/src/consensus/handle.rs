//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_consensus::hotstuff::{ConsensusCurrentState, CurrentView, HotstuffEvent};
use tari_transaction::Transaction;
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

    pub fn subscribe_to_hotstuff_events(&mut self) -> broadcast::Receiver<HotstuffEvent> {
        self.events_subscription.subscribe()
    }

    pub fn get_current_state(&self) -> ConsensusCurrentState {
        *self.rx_current_state.borrow()
    }

    pub fn is_running(&self) -> bool {
        self.get_current_state().is_running()
    }
}
