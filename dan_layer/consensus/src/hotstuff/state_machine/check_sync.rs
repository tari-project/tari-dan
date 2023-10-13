//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::marker::PhantomData;

use crate::{
    hotstuff::{
        state_machine::{event::ConsensusStateEvent, idle::Idle, running::Running, worker::ConsensusWorkerContext},
        HotStuffError,
    },
    traits::{ConsensusSpec, SyncManager, SyncStatus},
};

const _LOG_TARGET: &str = "tari::dan::consensus::sm::check_sync";

#[derive(Debug, Clone)]
pub struct CheckSync<TSpec>(PhantomData<TSpec>);

impl<TSpec> CheckSync<TSpec>
where
    TSpec: ConsensusSpec,
    HotStuffError: From<<TSpec::SyncManager as SyncManager>::Error>,
{
    pub(super) async fn on_enter(
        &self,
        context: &mut ConsensusWorkerContext<TSpec>,
    ) -> Result<ConsensusStateEvent, HotStuffError> {
        match context.state_sync.check_sync().await? {
            SyncStatus::UpToDate => Ok(ConsensusStateEvent::Ready),
            SyncStatus::Behind => Ok(ConsensusStateEvent::NeedSync),
        }
    }
}

impl<TSpec> From<Idle<TSpec>> for CheckSync<TSpec> {
    fn from(_: Idle<TSpec>) -> Self {
        Self(PhantomData)
    }
}

impl<TSpec> From<Running<TSpec>> for CheckSync<TSpec> {
    fn from(_: Running<TSpec>) -> Self {
        Self(PhantomData)
    }
}
