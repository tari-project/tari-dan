//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::marker::PhantomData;

use log::*;

use crate::{
    hotstuff::{
        state_machine::{
            event::ConsensusStateEvent,
            idle::IdleState,
            running::Running,
            worker::ConsensusWorkerContext,
        },
        HotStuffError,
    },
    traits::ConsensusSpec,
};

const LOG_TARGET: &str = "tari::dan::consensus::sm::check_sync";

#[derive(Debug, Clone)]
pub struct CheckSync<TSpec>(PhantomData<TSpec>);

impl<TSpec: ConsensusSpec> CheckSync<TSpec> {
    pub(super) async fn on_enter(
        &self,
        _context: &mut ConsensusWorkerContext<TSpec>,
    ) -> Result<ConsensusStateEvent, HotStuffError> {
        warn!(target: LOG_TARGET, "CheckSync not implemented");
        Ok(ConsensusStateEvent::Ready)
    }
}

impl<TSpec> From<IdleState<TSpec>> for CheckSync<TSpec> {
    fn from(_: IdleState<TSpec>) -> Self {
        Self(PhantomData)
    }
}

impl<TSpec> From<Running<TSpec>> for CheckSync<TSpec> {
    fn from(_: Running<TSpec>) -> Self {
        Self(PhantomData)
    }
}
