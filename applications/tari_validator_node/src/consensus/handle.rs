//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_consensus::hotstuff::{ConsensusCurrentState, HotstuffEvent};
use tokio::sync::{broadcast, watch};

use crate::event_subscription::EventSubscription;

#[derive(Debug, Clone)]
pub struct ConsensusHandle {
    rx_current_state: watch::Receiver<ConsensusCurrentState>,
    events_subscription: EventSubscription<HotstuffEvent>,
}

impl ConsensusHandle {
    pub(super) fn new(
        rx_current_state: watch::Receiver<ConsensusCurrentState>,
        events_subscription: EventSubscription<HotstuffEvent>,
    ) -> Self {
        Self {
            rx_current_state,
            events_subscription,
        }
    }

    pub fn subscribe_to_hotstuff_events(&mut self) -> broadcast::Receiver<HotstuffEvent> {
        self.events_subscription.subscribe()
    }

    pub fn get_current_state(&self) -> ConsensusCurrentState {
        *self.rx_current_state.borrow()
    }
}
