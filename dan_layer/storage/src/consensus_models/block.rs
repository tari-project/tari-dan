//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{BTreeSet, HashSet},
    fmt::{Debug, Display},
    ops::DerefMut,
};

use serde::{Deserialize, Serialize};
use tari_common_types::types::{FixedHash, FixedHashSizeError};
use tari_dan_common_types::{hashing, Epoch, NodeHeight, ShardId};

use super::{QuorumCertificate, TransactionDecision};
use crate::{
    consensus_models::{LastExecuted, LockedBlock, TransactionId},
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Block {
    // Header
    id: BlockId,
    parent: BlockId,
    justify: QuorumCertificate,
    height: NodeHeight,
    epoch: Epoch,
    round: u64,
    proposed_by: ShardId,

    // Body
    merkle_root: FixedHash,
    // BTreeSet is used for the deterministic block hash, that is, transactions are always ordered by TransactionId.
    prepared: BTreeSet<TransactionDecision>,
    precommitted: BTreeSet<TransactionDecision>,
    committed: BTreeSet<TransactionDecision>,
}

impl Block {
    pub fn new(
        parent: BlockId,
        justify: QuorumCertificate,
        height: NodeHeight,
        epoch: Epoch,
        round: u64,
        proposed_by: ShardId,
        prepared: BTreeSet<TransactionDecision>,
        precommitted: BTreeSet<TransactionDecision>,
        committed: BTreeSet<TransactionDecision>,
    ) -> Self {
        let mut block = Self {
            id: BlockId::genesis(),
            parent,
            justify,
            height,
            epoch,
            round,
            proposed_by,
            // TODO
            merkle_root: FixedHash::zero(),
            prepared,
            precommitted,
            committed,
        };
        block.id = block.calculate_hash().into();
        block
    }

    pub fn genesis(epoch: Epoch) -> Self {
        Self::new(
            BlockId::genesis(),
            QuorumCertificate::genesis(epoch),
            NodeHeight(0),
            epoch,
            0,
            ShardId::zero(),
            Default::default(),
            Default::default(),
            Default::default(),
        )
    }

    pub fn calculate_hash(&self) -> FixedHash {
        hashing::block_hasher()
            .chain(&self.parent)
            .chain(&self.justify)
            .chain(&self.height)
            .chain(&self.epoch)
            .chain(&self.proposed_by)
            .chain(&self.merkle_root)
            .chain(&self.prepared)
            .chain(&self.precommitted)
            .chain(&self.committed)
            .result()
    }

    pub fn all_transaction_decisions(&self) -> impl Iterator<Item = &TransactionDecision> + '_ {
        self.prepared
            .iter()
            .chain(self.precommitted.iter())
            .chain(self.committed.iter())
    }

    pub fn all_transaction_ids(&self) -> impl Iterator<Item = TransactionId> + '_ {
        self.all_transaction_decisions().map(|d| d.transaction_id)
    }

    pub fn transaction_count(&self) -> usize {
        self.prepared.len() + self.precommitted.len() + self.committed.len()
    }

    pub fn as_locked(&self) -> LockedBlock {
        LockedBlock {
            epoch: self.epoch,
            height: self.height,
            block_id: self.id,
        }
    }

    pub fn as_last_executed(&self) -> LastExecuted {
        LastExecuted {
            epoch: self.epoch,
            height: self.height,
            block_id: self.id,
        }
    }
}

// impl getters for Block
impl Block {
    pub fn id(&self) -> &BlockId {
        &self.id
    }

    pub fn parent(&self) -> &BlockId {
        &self.parent
    }

    pub fn justify(&self) -> &QuorumCertificate {
        &self.justify
    }

    pub fn height(&self) -> NodeHeight {
        self.height
    }

    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    pub fn round(&self) -> u64 {
        self.round
    }

    pub fn proposed_by(&self) -> &ShardId {
        &self.proposed_by
    }

    pub fn merkle_root(&self) -> &FixedHash {
        &self.merkle_root
    }

    pub fn prepared(&self) -> &BTreeSet<TransactionDecision> {
        &self.prepared
    }

    pub fn precommitted(&self) -> &BTreeSet<TransactionDecision> {
        &self.precommitted
    }

    pub fn committed(&self) -> &BTreeSet<TransactionDecision> {
        &self.committed
    }
}

impl Block {
    pub fn get<TTx: StateStoreReadTransaction>(tx: &mut TTx, id: &BlockId) -> Result<Self, StorageError> {
        tx.blocks_get(id)
    }

    pub fn exists<TTx: StateStoreReadTransaction + ?Sized>(&self, tx: &mut TTx) -> Result<bool, StorageError> {
        tx.blocks_exists(self.id())
    }

    pub fn insert<TTx: StateStoreWriteTransaction + ?Sized>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        tx.blocks_insert(self)
    }

    pub fn save<TTx>(&self, tx: &mut TTx) -> Result<(), StorageError>
    where
        TTx: StateStoreWriteTransaction + DerefMut,
        TTx::Target: StateStoreReadTransaction,
    {
        if self.exists(tx.deref_mut())? {
            return Ok(());
        }
        self.insert(tx)
    }

    pub fn find_involved_shards<TTx: StateStoreReadTransaction>(
        &self,
        tx: &mut TTx,
    ) -> Result<HashSet<ShardId>, StorageError> {
        tx.transaction_pools_fetch_involved_shards(self.all_transaction_ids().collect())
    }

    pub fn extends<TTx: StateStoreReadTransaction>(
        &self,
        tx: &mut TTx,
        ancestor: &BlockId,
    ) -> Result<bool, StorageError> {
        if self.parent == *ancestor {
            return Ok(true);
        }
        tx.blocks_is_ancestor(self.parent(), ancestor)
    }

    pub fn set_as_locked<TTx: StateStoreWriteTransaction>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        self.as_locked().set(tx)
    }

    pub fn set_as_last_executed<TTx: StateStoreWriteTransaction>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        self.as_last_executed().set(tx)
    }

    pub fn get_parent<TTx: StateStoreReadTransaction>(&self, tx: &mut TTx) -> Result<Block, StorageError> {
        Block::get(tx, &self.parent)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BlockId(FixedHash);

impl BlockId {
    pub const fn genesis() -> Self {
        Self(FixedHash::zero())
    }

    pub fn new<T: Into<FixedHash>>(hash: T) -> Self {
        Self(hash.into())
    }

    pub const fn hash(&self) -> &FixedHash {
        &self.0
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_slice()
    }

    pub fn is_genesis(&self) -> bool {
        self.0.iter().all(|b| *b == 0)
    }
}

impl AsRef<[u8]> for BlockId {
    fn as_ref(&self) -> &[u8] {
        self.0.as_slice()
    }
}

impl From<FixedHash> for BlockId {
    fn from(value: FixedHash) -> Self {
        Self(value)
    }
}

impl TryFrom<Vec<u8>> for BlockId {
    type Error = FixedHashSizeError;

    fn try_from(value: Vec<u8>) -> Result<Self, Self::Error> {
        FixedHash::try_from(value).map(Self)
    }
}

impl Display for BlockId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Display::fmt(&self.0, f)
    }
}
