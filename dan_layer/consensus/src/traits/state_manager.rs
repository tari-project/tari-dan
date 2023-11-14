//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use tari_dan_common_types::committee::CommitteeShard;
use tari_dan_storage::{
    consensus_models::{Block, ExecutedTransaction},
    StateStore,
};

pub trait StateManager<TStateStore: StateStore> {
    type Error: std::error::Error + Send + Sync + 'static;

    fn commit_transaction(
        &self,
        tx: &mut TStateStore::WriteTransaction<'_>,
        block: &Block<TStateStore::Addr>,
        transaction: &ExecutedTransaction,
        local_committee_shard: &CommitteeShard,
    ) -> Result<(), Self::Error>;
}
