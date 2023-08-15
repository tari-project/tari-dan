//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::borrow::Borrow;

use tari_dan_common_types::ShardId;
use tari_transaction::TransactionId;

use crate::{
    consensus_models::{BlockId, SubstateLockState},
    StateStoreWriteTransaction,
    StorageError,
};

#[derive(Debug, Clone)]
pub struct LockedOutput {
    pub block_id: BlockId,
    pub transaction_id: TransactionId,
    pub shard_id: ShardId,
}

impl LockedOutput {
    pub fn try_acquire_all<TTx, I, B>(
        tx: &mut TTx,
        block_id: &BlockId,
        transaction_id: &TransactionId,
        output_shards: I,
    ) -> Result<SubstateLockState, StorageError>
    where
        TTx: StateStoreWriteTransaction,
        I: IntoIterator<Item = B>,
        B: Borrow<ShardId>,
    {
        tx.locked_outputs_acquire_all(block_id, transaction_id, output_shards)
    }

    pub fn try_release_all<TTx, I, B>(tx: &mut TTx, output_shards: I) -> Result<Vec<Self>, StorageError>
    where
        TTx: StateStoreWriteTransaction,
        I: IntoIterator<Item = B>,
        B: Borrow<ShardId>,
    {
        tx.locked_outputs_release_all(output_shards)
    }
}
