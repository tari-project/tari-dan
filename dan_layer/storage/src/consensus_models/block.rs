//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{BTreeSet, HashSet},
    fmt::{Debug, Display},
    ops::{DerefMut, RangeInclusive},
};

use serde::{Deserialize, Serialize};
use tari_common_types::types::{FixedHash, FixedHashSizeError};
use tari_dan_common_types::{hashing, serde_with, Epoch, NodeAddressable, NodeHeight, ShardId};
use tari_transaction::TransactionId;

use super::QuorumCertificate;
use crate::{
    consensus_models::{Command, LastExecuted, LastProposed, LastVoted, LeafBlock, LockedBlock, Vote},
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Block<TAddr> {
    // Header
    id: BlockId,
    parent: BlockId,
    justify: QuorumCertificate<TAddr>,
    height: NodeHeight,
    epoch: Epoch,
    proposed_by: TAddr,
    total_leader_fee: u64,

    // Body
    merkle_root: FixedHash,
    // BTreeSet is used for the deterministic block hash, that is, transactions are always ordered by TransactionId.
    commands: BTreeSet<Command>,
}

impl<TAddr: NodeAddressable + Serialize> Block<TAddr> {
    pub fn new(
        parent: BlockId,
        justify: QuorumCertificate<TAddr>,
        height: NodeHeight,
        epoch: Epoch,
        proposed_by: TAddr,
        commands: BTreeSet<Command>,
        total_leader_fee: u64,
    ) -> Self {
        let mut block = Self {
            id: BlockId::genesis(),
            parent,
            justify,
            height,
            epoch,
            proposed_by,
            // TODO
            merkle_root: FixedHash::zero(),
            commands,
            total_leader_fee,
        };
        block.id = block.calculate_hash().into();
        block
    }

    pub fn load(
        id: BlockId,
        parent: BlockId,
        justify: QuorumCertificate<TAddr>,
        height: NodeHeight,
        epoch: Epoch,
        proposed_by: TAddr,
        commands: BTreeSet<Command>,
        total_leader_fee: u64,
    ) -> Self {
        Self {
            id,
            parent,
            justify,
            height,
            epoch,
            proposed_by,
            // TODO
            merkle_root: FixedHash::zero(),
            commands,
            total_leader_fee,
        }
    }

    pub fn genesis() -> Self {
        Self::new(
            BlockId::genesis(),
            QuorumCertificate::genesis(),
            NodeHeight(0),
            Epoch(0),
            TAddr::zero(),
            Default::default(),
            0,
        )
    }

    /// This is the parent block for all genesis blocks. Its block ID is always zero.
    pub fn zero_block() -> Self {
        Self {
            id: BlockId::genesis(),
            parent: BlockId::genesis(),
            justify: QuorumCertificate::genesis(),
            height: NodeHeight(0),
            epoch: Epoch(0),
            proposed_by: TAddr::zero(),
            merkle_root: FixedHash::zero(),
            commands: Default::default(),
            total_leader_fee: 0,
        }
    }

    pub fn dummy_block(parent: BlockId, proposed_by: TAddr, node_height: NodeHeight, epoch: Epoch) -> Self {
        Self::new(
            parent,
            QuorumCertificate::genesis(),
            node_height,
            epoch,
            proposed_by,
            Default::default(),
            0,
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
            .chain(&self.commands)
            .result()
    }
}

impl<TAddr> Block<TAddr> {
    pub fn is_genesis(&self) -> bool {
        self.parent == BlockId::genesis()
    }

    pub fn all_transaction_ids(&self) -> impl Iterator<Item = &TransactionId> + '_ {
        self.commands.iter().map(|d| d.transaction_id())
    }

    pub fn command_count(&self) -> usize {
        self.commands.len()
    }

    pub fn as_locked(&self) -> LockedBlock {
        LockedBlock {
            height: self.height,
            block_id: self.id,
        }
    }

    pub fn as_last_executed(&self) -> LastExecuted {
        LastExecuted {
            height: self.height,
            block_id: self.id,
        }
    }

    pub fn as_last_voted(&self) -> LastVoted {
        LastVoted {
            height: self.height,
            block_id: self.id,
        }
    }

    pub fn as_leaf_block(&self) -> LeafBlock {
        LeafBlock {
            height: self.height,
            block_id: self.id,
        }
    }

    pub fn as_last_proposed(&self) -> LastProposed {
        LastProposed {
            height: self.height,
            block_id: self.id,
        }
    }

    pub fn id(&self) -> &BlockId {
        &self.id
    }

    pub fn parent(&self) -> &BlockId {
        &self.parent
    }

    pub fn justify(&self) -> &QuorumCertificate<TAddr> {
        &self.justify
    }

    pub fn height(&self) -> NodeHeight {
        self.height
    }

    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    pub fn total_leader_fee(&self) -> u64 {
        self.total_leader_fee
    }

    pub fn proposed_by(&self) -> &TAddr {
        &self.proposed_by
    }

    pub fn merkle_root(&self) -> &FixedHash {
        &self.merkle_root
    }

    pub fn commands(&self) -> &BTreeSet<Command> {
        &self.commands
    }

    pub fn into_commands(self) -> BTreeSet<Command> {
        self.commands
    }
}

impl<TAddr: NodeAddressable> Block<TAddr> {
    pub fn get<TTx: StateStoreReadTransaction<Addr = TAddr> + ?Sized>(
        tx: &mut TTx,
        id: &BlockId,
    ) -> Result<Self, StorageError> {
        tx.blocks_get(id)
    }

    pub fn get_tip<TTx: StateStoreReadTransaction<Addr = TAddr>>(tx: &mut TTx) -> Result<Self, StorageError> {
        tx.blocks_get_tip()
    }

    pub fn exists<TTx: StateStoreReadTransaction<Addr = TAddr> + ?Sized>(
        &self,
        tx: &mut TTx,
    ) -> Result<bool, StorageError> {
        tx.blocks_exists(self.id())
    }

    pub fn insert<TTx: StateStoreWriteTransaction<Addr = TAddr> + ?Sized>(
        &self,
        tx: &mut TTx,
    ) -> Result<(), StorageError> {
        tx.blocks_insert(self)
    }

    /// Inserts the block if it doesnt exist. Returns true if the block exists, otherwise false.
    pub fn save<TTx>(&self, tx: &mut TTx) -> Result<bool, StorageError>
    where
        TTx: StateStoreWriteTransaction<Addr = TAddr> + DerefMut,
        TTx::Target: StateStoreReadTransaction<Addr = TAddr>,
    {
        let exists = self.exists(tx.deref_mut())?;
        if exists {
            return Ok(true);
        }
        self.insert(tx)?;
        Ok(false)
    }

    pub fn commit<TTx: StateStoreWriteTransaction<Addr = TAddr>>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        tx.blocks_commit(self.id())
    }

    pub fn find_involved_shards<TTx: StateStoreReadTransaction<Addr = TAddr>>(
        &self,
        tx: &mut TTx,
    ) -> Result<HashSet<ShardId>, StorageError> {
        tx.transactions_fetch_involved_shards(self.all_transaction_ids().copied().collect())
    }

    pub fn extends<TTx: StateStoreReadTransaction<Addr = TAddr>>(
        &self,
        tx: &mut TTx,
        ancestor: &BlockId,
    ) -> Result<bool, StorageError> {
        if self.parent == *ancestor {
            return Ok(true);
        }
        tx.blocks_is_ancestor(self.parent(), ancestor)
    }

    pub fn get_parent<TTx: StateStoreReadTransaction<Addr = TAddr>>(
        &self,
        tx: &mut TTx,
    ) -> Result<Block<TAddr>, StorageError> {
        Block::get(tx, &self.parent)
    }

    pub fn get_votes<TTx: StateStoreReadTransaction<Addr = TAddr>>(
        &self,
        tx: &mut TTx,
    ) -> Result<Vec<Vote<TAddr>>, StorageError> {
        Vote::get_for_block(tx, &self.id)
    }

    pub fn get_child<TTx: StateStoreReadTransaction<Addr = TAddr>>(&self, tx: &mut TTx) -> Result<Self, StorageError> {
        tx.blocks_get_by_parent(self.id())
    }

    pub fn get_total_due_for_epoch<TTx: StateStoreReadTransaction<Addr = TAddr>>(
        tx: &mut TTx,
        epoch: Epoch,
        validator_public_key: &TAddr,
    ) -> Result<u64, StorageError> {
        tx.blocks_get_total_leader_fee_for_epoch(epoch, validator_public_key)
    }

    pub fn get_any_with_epoch_range_for_validator<TTx: StateStoreReadTransaction<Addr = TAddr>>(
        tx: &mut TTx,
        range: RangeInclusive<Epoch>,
        validator_public_key: Option<&TAddr>,
    ) -> Result<Vec<Self>, StorageError> {
        tx.blocks_get_any_with_epoch_range(range, validator_public_key)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BlockId(#[serde(with = "serde_with::hex")] FixedHash);

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
        Self::try_from(value.as_slice())
    }
}

impl TryFrom<&[u8]> for BlockId {
    type Error = FixedHashSizeError;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        FixedHash::try_from(value).map(Self)
    }
}

impl Display for BlockId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Display::fmt(&self.0, f)
    }
}
