//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use tari_dan_storage::{consensus_models::ExecutedTransaction, StateStore};

pub trait StateManager<TStateStore: StateStore> {
    type Error: std::error::Error + Send + Sync + 'static;

    fn commit_transaction(
        &self,
        tx: &mut TStateStore::WriteTransaction<'_>,
        transaction: &ExecutedTransaction,
    ) -> Result<(), Self::Error>;
}
