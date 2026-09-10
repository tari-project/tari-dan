//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use log::*;

use crate::{
    hotstuff::{
        HotStuffError,
        WorkerExitReason,
        state_machine::{
            check_sync::CheckSync,
            event::ConsensusStateEvent,
            syncing::Syncing,
            worker::ConsensusWorkerContext,
        },
    },
    traits::ConsensusSpec,
};

const LOG_TARGET: &str = "tari::ootle::consensus::sm::running";

#[derive(Debug)]
pub(super) struct Running<TSpec> {
    _phantom: std::marker::PhantomData<TSpec>,
}

impl<TSpec> Running<TSpec>
where TSpec: ConsensusSpec
{
    pub(super) async fn on_enter(
        &self,
        context: &mut ConsensusWorkerContext<TSpec>,
    ) -> Result<ConsensusStateEvent, HotStuffError> {
        match context.hotstuff.start().await {
            Ok(WorkerExitReason::Shutdown) => {
                info!(target: LOG_TARGET, "HotStuff shut down");
                Ok(ConsensusStateEvent::Shutdown)
            },
            Err(ref err @ HotStuffError::NotRegisteredForCurrentEpoch { epoch }) => {
                info!(target: LOG_TARGET, "Not registered for current epoch ({err})");
                Ok(ConsensusStateEvent::NotRegisteredForEpoch { epoch })
            },
            Err(err) if err.is_sync_required() => {
                info!(target: LOG_TARGET, "⚠️ Behind peers, starting sync ({err})");
                // From the Running state we have no specific target — re-enter CheckSync to
                // resolve one via the probe/oracle.
                Ok(ConsensusStateEvent::NeedSync { target_epoch: None })
            },
            Err(err) => {
                error!(target: LOG_TARGET, "HotStuff crashed: {}", err);
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
