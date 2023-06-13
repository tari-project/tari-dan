//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashMap, ops::Deref};

use tari_common_types::types::FixedHash;
use tari_dan_common_types::{hasher::TariHasher, hashing::pledge_hasher, ShardId};

use crate::{
    consensus_models::{BlockId, TransactionId},
    StateStoreWriteTransaction, StorageError,
};

#[derive(Debug, Clone)]
pub struct Pledge {
    pub shard_id: ShardId,
    pub created_by_block: BlockId,
    pub pledged_to_transaction_id: TransactionId,
    pub is_active: bool,
    pub completed_by_block: Option<BlockId>,
    pub abandoned_by_block: Option<BlockId>,
    pub state_hash: FixedHash,
}

impl Pledge {
    pub fn new(
        shard_id: ShardId,
        created_by_block: BlockId,
        pledged_to_transaction_id: TransactionId,
        state_hash: FixedHash,
    ) -> Self {
        Self {
            shard_id,
            created_by_block,
            pledged_to_transaction_id,
            is_active: true,
            completed_by_block: None,
            abandoned_by_block: None,
            state_hash,
        }
    }
}

/// An ordered list of Pledges.
#[derive(Debug, Clone)]
pub struct PledgeCollection {
    block_id: BlockId,
    pledges: Vec<Pledge>,
    pledge_hash: FixedHash,
}

impl PledgeCollection {
    pub fn new(block_id: BlockId, mut pledges: Vec<Pledge>) -> Self {
        pledges.sort_by_key(|p| p.shard_id);
        let pledge_hash = hash_pledges(&block_id, &pledges);
        Self {
            block_id,
            pledges,
            pledge_hash,
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &Pledge> {
        self.pledges.iter()
    }

    pub fn pledge_hash(&self) -> FixedHash {
        self.pledge_hash
    }

    pub fn block_id(&self) -> &BlockId {
        &self.block_id
    }
}

fn hash_pledges(block_id: &BlockId, pledges: &[Pledge]) -> FixedHash {
    let hasher = pledge_hasher().chain(block_id);
    pledges
        .iter()
        .map(|p| &p.state_hash)
        .fold(hasher, TariHasher::chain)
        .result()
}

impl Deref for PledgeCollection {
    type Target = [Pledge];

    fn deref(&self) -> &Self::Target {
        &self.pledges
    }
}

impl PledgeCollection {
    pub fn pledge_many<TTx: StateStoreWriteTransaction>(
        tx: &mut TTx,
        block_id: &BlockId,
        transactions_and_shards: HashMap<TransactionId, Vec<ShardId>>,
    ) -> Result<Self, StorageError> {
        tx.create_pledges(block_id, transactions_and_shards)
    }
}
