//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use tari_consensus::traits::StateManager;
use tari_dan_storage::{consensus_models::ExecutedTransaction, StateStore};

pub struct NoopStateManager;

impl NoopStateManager {
    pub fn new() -> Self {
        Self
    }
}

impl<TStateStore: StateStore> StateManager<TStateStore> for NoopStateManager {
    type Error = NoopStateManagerError;

    fn commit_transaction(
        &self,
        _tx: &mut TStateStore::WriteTransaction<'_>,
        _transaction: &ExecutedTransaction,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
#[error("NoopStateManagerError")]
pub struct NoopStateManagerError;
