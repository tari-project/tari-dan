//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::marker::PhantomData;

use crate::{
    hotstuff::{
        state_machine::{check_sync::CheckSync, event::ConsensusStateEvent},
        ConsensusWorkerContext,
        HotStuffError,
    },
    traits::ConsensusSpec,
};

#[derive(Debug)]
pub struct Syncing<TSpec>(PhantomData<TSpec>);

impl<TSpec: ConsensusSpec> Syncing<TSpec> {
    pub(super) async fn on_enter(
        &self,
        _context: &mut ConsensusWorkerContext<TSpec>,
    ) -> Result<ConsensusStateEvent, HotStuffError> {
        // let mut sync = SyncWorker::new(context);
        // sync.start().await?;
        Ok(ConsensusStateEvent::SyncComplete)
    }
}

impl<TSpec> From<CheckSync<TSpec>> for Syncing<TSpec> {
    fn from(_: CheckSync<TSpec>) -> Self {
        Self(PhantomData)
    }
}
