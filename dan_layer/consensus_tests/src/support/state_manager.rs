//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use std::sync::{atomic::AtomicBool, Arc};

use tari_consensus::traits::StateManager;
use tari_dan_common_types::committee::CommitteeShard;
use tari_dan_storage::{
    consensus_models::{Block, ExecutedTransaction},
    StateStore,
};

#[derive(Debug, Clone)]
pub struct NoopStateManager(Arc<AtomicBool>);

impl NoopStateManager {
    pub fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }

    pub fn is_committed(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::Relaxed)
    }

    #[allow(dead_code)]
    pub fn reset(&self) {
        self.0.store(false, std::sync::atomic::Ordering::Relaxed);
    }
}

impl<TStateStore: StateStore> StateManager<TStateStore> for NoopStateManager {
    type Error = NoopStateManagerError;

    fn commit_transaction(
        &self,
        _tx: &mut TStateStore::WriteTransaction<'_>,
        _block: &Block,
        _transaction: &ExecutedTransaction,
        _local_committee_shard: &CommitteeShard,
    ) -> Result<(), Self::Error> {
        self.0.store(true, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
#[error("NoopStateManagerError")]
pub struct NoopStateManagerError;
