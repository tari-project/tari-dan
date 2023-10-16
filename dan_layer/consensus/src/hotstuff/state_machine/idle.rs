//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::marker::PhantomData;

use log::*;
use tari_dan_common_types::Epoch;
use tari_epoch_manager::{EpochManagerEvent, EpochManagerReader};
use tokio::sync::broadcast;

use crate::{
    hotstuff::{
        state_machine::{event::ConsensusStateEvent, running::Running, worker::ConsensusWorkerContext},
        HotStuffError,
    },
    traits::ConsensusSpec,
};

const LOG_TARGET: &str = "tari::dan::consensus::sm::idle";

#[derive(Debug, Clone)]
pub struct Idle<TSpec>(PhantomData<TSpec>);

impl<TSpec> Idle<TSpec>
where TSpec: ConsensusSpec
{
    pub fn new() -> Self {
        Self(PhantomData)
    }

    pub(super) async fn on_enter(
        &self,
        context: &mut ConsensusWorkerContext<TSpec>,
    ) -> Result<ConsensusStateEvent, HotStuffError> {
        // Subscribe before checking if we're registered to eliminate the chance that we miss the epoch event
        let mut epoch_events = context.epoch_manager.subscribe().await?;
        let current_epoch = context.epoch_manager.current_epoch().await?;
        if self.is_registered_for_epoch(context, current_epoch).await? {
            return Ok(ConsensusStateEvent::RegisteredForEpoch { epoch: current_epoch });
        }

        loop {
            tokio::select! {
                event = epoch_events.recv() => {
                    match event {
                        Ok(event) => {
                            if let Some(event) = self.on_epoch_event(context, event).await? {
                                return Ok(event);
                            }
                        },
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            debug!(target: LOG_TARGET, "Idle state lagged behind epoch manager event stream");
                        },
                        Err(broadcast::error::RecvError::Closed) => {
                            break;
                        },
                    }
                },
                // Ignore hotstuff messages while idle
                _ = context.hotstuff.discard_messages() => { }
            }
        }

        debug!(target: LOG_TARGET, "Idle event triggering shutdown because epoch manager event stream closed");
        Ok(ConsensusStateEvent::Shutdown)
    }

    async fn is_registered_for_epoch(
        &self,
        context: &mut ConsensusWorkerContext<TSpec>,
        epoch: Epoch,
    ) -> Result<bool, HotStuffError> {
        let is_registered = context
            .epoch_manager
            .is_this_validator_registered_for_epoch(epoch)
            .await?;
        Ok(is_registered)
    }

    async fn on_epoch_event(
        &self,
        context: &mut ConsensusWorkerContext<TSpec>,
        event: EpochManagerEvent,
    ) -> Result<Option<ConsensusStateEvent>, HotStuffError> {
        match event {
            EpochManagerEvent::EpochChanged(epoch) => {
                if self.is_registered_for_epoch(context, epoch).await? {
                    Ok(Some(ConsensusStateEvent::RegisteredForEpoch { epoch }))
                } else {
                    Ok(None)
                }
            },
            EpochManagerEvent::ThisValidatorIsRegistered { .. } => Ok(None),
        }
    }
}

impl<TSpec: ConsensusSpec> From<Running<TSpec>> for Idle<TSpec> {
    fn from(_value: Running<TSpec>) -> Self {
        Idle::new()
    }
}
