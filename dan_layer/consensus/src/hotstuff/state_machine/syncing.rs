//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::marker::PhantomData;

use crate::{
    hotstuff::{
        state_machine::{check_sync::CheckSync, event::ConsensusStateEvent},
        ConsensusWorkerContext,
        HotStuffError,
    },
    traits::{ConsensusSpec, SyncManager},
};

#[derive(Debug)]
pub struct Syncing<TSpec>(PhantomData<TSpec>);

impl<TSpec> Syncing<TSpec>
where
    TSpec: ConsensusSpec,
    HotStuffError: From<<TSpec::SyncManager as SyncManager>::Error>,
{
    pub(super) async fn on_enter(
        &self,
        context: &mut ConsensusWorkerContext<TSpec>,
    ) -> Result<ConsensusStateEvent, HotStuffError> {
        context.state_sync.sync().await?;
        Ok(ConsensusStateEvent::SyncComplete)
    }
}

impl<TSpec> From<CheckSync<TSpec>> for Syncing<TSpec> {
    fn from(_: CheckSync<TSpec>) -> Self {
        Self(PhantomData)
    }
}
