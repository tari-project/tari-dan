//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use log::*;

use crate::{
    hotstuff::{
        state_machine::{
            check_sync::CheckSync,
            event::ConsensusStateEvent,
            syncing::Syncing,
            worker::ConsensusWorkerContext,
        },
        HotStuffError,
        ProposalValidationError,
    },
    traits::ConsensusSpec,
};

const LOG_TARGET: &str = "tari::dan::consensus::sm::running";

#[derive(Debug)]
pub(super) struct Running<TSpec> {
    _phantom: std::marker::PhantomData<TSpec>,
}

impl<TSpec> Running<TSpec>
where
    TSpec: ConsensusSpec
{
    pub(super) async fn on_enter(
        &self,
        context: &mut ConsensusWorkerContext<TSpec>,
    ) -> Result<ConsensusStateEvent, HotStuffError> {
        match context.hotstuff.start().await {
            Ok(_) => {
                info!(target: LOG_TARGET, "HotStuff shut down");
                Ok(ConsensusStateEvent::Shutdown)
            },
            Err(ref err @ HotStuffError::NotRegisteredForCurrentEpoch { epoch }) => {
                info!(target: LOG_TARGET, "Not registered for current epoch ({err})");
                Ok(ConsensusStateEvent::NotRegisteredForEpoch { epoch })
            },
            Err(err @ HotStuffError::ProposalValidationError(ProposalValidationError::JustifyBlockNotFound { .. })) |
            Err(err @ HotStuffError::FallenBehind { .. }) => {
                info!(target: LOG_TARGET, "Behind peers, starting sync ({err})");
                Ok(ConsensusStateEvent::NeedSync)
            },
            Err(err) => {
                error!(target: LOG_TARGET, "HotStuff failed to start: {}", err);
                Err(err)
            },
        }
    }
}

impl<TSpec> From<CheckSync<TSpec>> for Running<TSpec> {
    fn from(_: CheckSync<TSpec>) -> Self {
        Self {
            _phantom: std::marker::PhantomData,
        }
    }
}

impl<TSpec> From<Syncing<TSpec>> for Running<TSpec> {
    fn from(_: Syncing<TSpec>) -> Self {
        Self {
            _phantom: std::marker::PhantomData,
        }
    }
}
