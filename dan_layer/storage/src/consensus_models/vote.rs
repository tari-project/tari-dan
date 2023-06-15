//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::ops::DerefMut;

use serde::{Deserialize, Serialize};
use tari_common_types::types::FixedHash;
use tari_dan_common_types::{
    hashing::{vote_hasher, ValidatorNodeMerkleProof},
    Epoch,
    ShardId,
};

use crate::{
    consensus_models::{BlockId, QuorumDecision, ValidatorSignature},
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Vote {
    pub epoch: Epoch,
    pub block_id: BlockId,
    pub decision: QuorumDecision,
    pub sender: ShardId,
    pub signature: ValidatorSignature,
    pub merkle_proof: ValidatorNodeMerkleProof,
}

impl Vote {
    pub fn calculate_hash(&self) -> FixedHash {
        vote_hasher().chain(self).result()
    }

    pub fn signature(&self) -> &ValidatorSignature {
        &self.signature
    }
}

impl Vote {
    // pub fn get<TTx: StateStoreReadTransaction>(tx: &mut TTx, epoch: Epoch) -> Result<Self, StorageError> {
    //     tx.last_voted_get(epoch)
    // }

    pub fn exists<TTx: StateStoreReadTransaction + ?Sized>(&self, tx: &mut TTx) -> Result<bool, StorageError> {
        let count = tx.votes_count_for_block(&self.block_id)?;
        Ok(count > 0)
    }

    pub fn save<TTx>(&self, tx: &mut TTx) -> Result<bool, StorageError>
    where
        TTx: StateStoreWriteTransaction + DerefMut,
        TTx::Target: StateStoreReadTransaction,
    {
        let exists = self.exists(tx.deref_mut())?;
        if !exists {
            self.insert(tx)?;
        }
        Ok(exists)
    }

    pub fn insert<TTx>(&self, tx: &mut TTx) -> Result<(), StorageError>
    where
        TTx: StateStoreWriteTransaction + DerefMut,
        TTx::Target: StateStoreReadTransaction,
    {
        tx.votes_insert(self)
    }

    pub fn count_for_block<TTx: StateStoreReadTransaction>(
        tx: &mut TTx,
        block_id: &BlockId,
    ) -> Result<usize, StorageError> {
        tx.votes_count_for_block(block_id).map(|v| v as usize)
    }

    pub fn get_for_block<TTx: StateStoreReadTransaction>(
        tx: &mut TTx,
        block_id: &BlockId,
    ) -> Result<Vec<Self>, StorageError> {
        tx.votes_get_for_block(block_id)
    }
}
