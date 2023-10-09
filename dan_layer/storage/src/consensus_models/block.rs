//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{BTreeSet, HashSet},
    fmt::{Debug, Display, Formatter},
    ops::{DerefMut, RangeInclusive},
};

use log::*;
use serde::{Deserialize, Serialize};
use tari_common_types::types::{FixedHash, FixedHashSizeError};
use tari_dan_common_types::{hashing, optional::Optional, serde_with, Epoch, NodeAddressable, NodeHeight, ShardId};
use tari_transaction::TransactionId;
use time::PrimitiveDateTime;

use super::QuorumCertificate;
use crate::{
    consensus_models::{
        Command,
        HighQc,
        LastExecuted,
        LastProposed,
        LastVoted,
        LeafBlock,
        LockedBlock,
        SubstateCreatedProof,
        SubstateUpdate,
        TransactionRecord,
        Vote,
    },
    Ordering,
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
};

const LOG_TARGET: &str = "tari::dan::storage::consensus_models::block";

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
    /// If the block is a dummy block. This is metadata and not sent over
    /// the wire or part of the block hash.
    is_dummy: bool,
    /// Flag that indicates that the block locked objects and made transaction stage transitions.
    is_processed: bool,
    /// Flag that indicates that the block has been committed.
    is_committed: bool,
    /// Timestamp when was this stored.
    stored_at: Option<PrimitiveDateTime>,
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
            is_dummy: false,
            is_processed: false,
            is_committed: false,
            stored_at: None,
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
        is_dummy: bool,
        is_processed: bool,
        is_committed: bool,
        created_at: PrimitiveDateTime,
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
            is_dummy,
            is_processed,
            is_committed,
            stored_at: Some(created_at),
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
            is_dummy: false,
            is_processed: false,
            is_committed: true,
            stored_at: None,
        }
    }

    pub fn dummy_block(
        parent: BlockId,
        proposed_by: TAddr,
        node_height: NodeHeight,
        high_qc: QuorumCertificate<TAddr>,
        epoch: Epoch,
    ) -> Self {
        let mut block = Self::new(parent, high_qc, node_height, epoch, proposed_by, Default::default(), 0);
        block.is_dummy = true;
        block.is_processed = false;
        block
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
        self.id.is_genesis()
    }

    pub fn all_transaction_ids(&self) -> impl Iterator<Item = &TransactionId> + '_ {
        self.commands.iter().map(|d| d.transaction_id())
    }

    pub fn command_count(&self) -> usize {
        self.commands.len()
    }

    pub fn as_locked_block(&self) -> LockedBlock {
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

    pub fn is_dummy(&self) -> bool {
        self.is_dummy
    }

    pub fn is_processed(&self) -> bool {
        self.is_processed
    }

    pub fn is_committed(&self) -> bool {
        self.is_committed
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

    pub fn get_all_blocks_after<TTx: StateStoreReadTransaction<Addr = TAddr>>(
        tx: &mut TTx,
        block_id: &BlockId,
    ) -> Result<Vec<Self>, StorageError> {
        tx.blocks_all_after(block_id)
    }

    pub fn exists<TTx: StateStoreReadTransaction<Addr = TAddr> + ?Sized>(
        &self,
        tx: &mut TTx,
    ) -> Result<bool, StorageError> {
        Self::record_exists(tx, self.id())
    }

    pub fn has_been_processed<TTx: StateStoreReadTransaction<Addr = TAddr> + ?Sized>(
        tx: &mut TTx,
        block_id: &BlockId,
    ) -> Result<bool, StorageError> {
        // TODO: consider optimising
        let is_processed = Self::get(tx, block_id)
            .optional()?
            .map(|b| b.is_processed())
            .unwrap_or(false);
        Ok(is_processed)
    }

    pub fn record_exists<TTx: StateStoreReadTransaction<Addr = TAddr> + ?Sized>(
        tx: &mut TTx,
        block_id: &BlockId,
    ) -> Result<bool, StorageError> {
        tx.blocks_exists(block_id)
    }

    pub fn insert<TTx: StateStoreWriteTransaction<Addr = TAddr> + ?Sized>(
        &self,
        tx: &mut TTx,
    ) -> Result<(), StorageError> {
        tx.blocks_insert(self)
    }

    pub fn get_paginated<TTx: StateStoreReadTransaction<Addr = TAddr>>(
        tx: &mut TTx,
        limit: u64,
        offset: u64,
        ordering: Option<Ordering>,
    ) -> Result<Vec<Self>, StorageError> {
        tx.blocks_get_paginated(limit, offset, ordering)
    }

    pub fn get_count<TTx: StateStoreReadTransaction<Addr = TAddr>>(tx: &mut TTx) -> Result<i64, StorageError> {
        tx.blocks_get_count()
    }

    /// Inserts the block if it doesnt exist. Returns true if the block was saved and did not exist previously,
    /// otherwise false.
    pub fn save<TTx>(&self, tx: &mut TTx) -> Result<bool, StorageError>
    where
        TTx: StateStoreWriteTransaction<Addr = TAddr> + DerefMut,
        TTx::Target: StateStoreReadTransaction<Addr = TAddr>,
    {
        let exists = self.exists(tx.deref_mut())?;
        if exists {
            return Ok(false);
        }
        self.insert(tx)?;
        Ok(true)
    }

    pub fn commit<TTx: StateStoreWriteTransaction<Addr = TAddr>>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        tx.blocks_set_flags(self.id(), Some(true), None)
    }

    pub fn set_as_processed<TTx: StateStoreWriteTransaction<Addr = TAddr>>(
        &self,
        tx: &mut TTx,
    ) -> Result<(), StorageError> {
        tx.blocks_set_flags(self.id(), None, Some(true))
    }

    pub fn unset_as_processed<TTx: StateStoreWriteTransaction<Addr = TAddr>>(
        &self,
        tx: &mut TTx,
    ) -> Result<(), StorageError> {
        tx.blocks_set_flags(self.id(), None, Some(false))
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
        if self.id == *ancestor {
            return Ok(false);
        }
        if self.parent == *ancestor {
            return Ok(true);
        }
        // First check the parent here, if it does not exist, then this block cannot extend anything.
        if !Block::record_exists(tx, self.parent())? {
            return Ok(false);
        }

        tx.blocks_is_ancestor(self.parent(), ancestor)
    }

    pub fn get_parent<TTx: StateStoreReadTransaction<Addr = TAddr>>(
        &self,
        tx: &mut TTx,
    ) -> Result<Block<TAddr>, StorageError> {
        if self.id.is_genesis() {
            return Err(StorageError::NotFound {
                item: "Block".to_string(),
                key: self.id.to_string(),
            });
        }
        Block::get(tx, &self.parent)
    }

    pub fn get_parent_chain<TTx: StateStoreReadTransaction<Addr = TAddr>>(
        &self,
        tx: &mut TTx,
        limit: usize,
    ) -> Result<Vec<Block<TAddr>>, StorageError> {
        tx.blocks_get_parent_chain(self.id(), limit)
    }

    pub fn get_votes<TTx: StateStoreReadTransaction<Addr = TAddr>>(
        &self,
        tx: &mut TTx,
    ) -> Result<Vec<Vote<TAddr>>, StorageError> {
        Vote::get_for_block(tx, &self.id)
    }

    pub fn get_child_blocks<TTx: StateStoreReadTransaction<Addr = TAddr>>(
        &self,
        tx: &mut TTx,
    ) -> Result<Vec<Self>, StorageError> {
        tx.blocks_get_all_by_parent(self.id())
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

    pub fn get_transactions<TTx: StateStoreReadTransaction<Addr = TAddr>>(
        &self,
        tx: &mut TTx,
    ) -> Result<Vec<TransactionRecord>, StorageError> {
        let tx_ids = self.commands().iter().map(|t| t.transaction_id());
        let (found, missing) = TransactionRecord::get_any(tx, tx_ids)?;
        if !missing.is_empty() {
            return Err(StorageError::NotFound {
                item: "Transaction".to_string(),
                key: missing
                    .into_iter()
                    .map(|id| id.to_string())
                    .collect::<Vec<_>>()
                    .join(", "),
            });
        }

        Ok(found)
    }

    pub fn get_substate_updates<TTx: StateStoreReadTransaction<Addr = TAddr>>(
        &self,
        tx: &mut TTx,
    ) -> Result<Vec<SubstateUpdate<TAddr>>, StorageError> {
        let committed = self
            .commands()
            .iter()
            .filter_map(|c| c.accept())
            .filter(|t| t.decision.is_commit())
            .collect::<Vec<_>>();

        let mut updates = Vec::with_capacity(committed.len());
        for transaction in committed {
            let substates = tx.substates_get_all_for_transaction(&transaction.id)?;
            for substate in substates {
                if let Some(destroyed) = substate.destroyed() {
                    // This substate is destroyed. One of the following are possible:
                    // 1. The substate was destroyed by this transaction and created in an earlier transaction
                    // 2. The substate was created by this transaction and destroyed in a later transaction
                    // It isn't possible for a substate to be created and destroyed by the same transaction
                    // because the engine can never emit such a substate diff.
                    if substate.created_by_transaction == transaction.id {
                        updates.push(SubstateUpdate::Create(SubstateCreatedProof {
                            created_qc: substate.get_created_quorum_certificate(tx)?,
                            substate: substate.into(),
                        }));
                    } else {
                        updates.push(SubstateUpdate::Destroy {
                            shard_id: substate.to_shard_id(),
                            proof: QuorumCertificate::get(tx, &destroyed.justify)?,
                            destroyed_by_transaction: destroyed.by_transaction,
                        });
                    }
                } else {
                    updates.push(SubstateUpdate::Create(SubstateCreatedProof {
                        created_qc: substate.get_created_quorum_certificate(tx)?,
                        substate: substate.into(),
                    }));
                };
            }
        }

        Ok(updates)
    }

    pub fn update_nodes<TTx, TFnOnLock, TFnOnCommit, E>(
        &self,
        tx: &mut TTx,
        on_lock_block: TFnOnLock,
        on_commit: TFnOnCommit,
    ) -> Result<HighQc, E>
    where
        TTx: StateStoreWriteTransaction<Addr = TAddr> + DerefMut + ?Sized,
        TTx::Target: StateStoreReadTransaction<Addr = TAddr>,
        TFnOnLock: FnOnce(&mut TTx, &LockedBlock, &Block<TAddr>) -> Result<(), E>,
        TFnOnCommit: FnOnce(&mut TTx, &LastExecuted, &Block<TAddr>) -> Result<(), E>,
        E: From<StorageError>,
    {
        let high_qc = self.justify().update_high_qc(tx)?;

        // b'' <- b*.justify.node
        let Some(commit_node) = self.justify().get_block(tx.deref_mut()).optional()? else {
            return Ok(high_qc);
        };

        // b' <- b''.justify.node
        let Some(precommit_node) = commit_node.justify().get_block(tx.deref_mut()).optional()? else {
            return Ok(high_qc);
        };

        let locked_block = LockedBlock::get(tx.deref_mut())?;
        if precommit_node.height() > locked_block.height {
            on_lock_block(tx, &locked_block, &precommit_node)?;
            precommit_node.as_locked_block().set(tx)?;
        }

        // b <- b'.justify.node
        let prepare_node = precommit_node.justify().block_id();
        if commit_node.parent() == precommit_node.id() && precommit_node.parent() == prepare_node {
            debug!(
                target: LOG_TARGET,
                "✅ Node {} {} forms a 3-chain b'' = {}, b' = {}, b = {}",
                self.height(),
                self.id(),
                commit_node.id(),
                precommit_node.id(),
                prepare_node,
            );

            // Commit prepare_node (b)
            let prepare_node = Block::get(tx.deref_mut(), prepare_node)?;
            let last_executed = LastExecuted::get(tx.deref_mut())?;
            on_commit(tx, &last_executed, &prepare_node)?;
            prepare_node.as_last_executed().set(tx)?;
        } else {
            debug!(
                target: LOG_TARGET,
                "Node {} {} DOES NOT form a 3-chain b'' = {}, b' = {}, b = {}, b* = {}",
                self.height(),
                self.id(),
                commit_node.id(),
                precommit_node.id(),
                prepare_node,
                self.id()
            );
        }

        Ok(high_qc)
    }

    /// safeNode predicate (https://arxiv.org/pdf/1803.05069v6.pdf)
    ///
    /// The safeNode predicate is a core ingredient of the protocol. It examines a proposal message
    /// m carrying a QC justification m.justify, and determines whether m.node is safe to accept. The safety rule to
    /// accept a proposal is the branch of m.node extends from the currently locked node lockedQC.node. On the other
    /// hand, the liveness rule is the replica will accept m if m.justify has a higher view than the current
    /// lockedQC. The predicate is true as long as either one of two rules holds.
    pub fn is_safe<TTx: StateStoreReadTransaction<Addr = TAddr>>(&self, tx: &mut TTx) -> Result<bool, StorageError> {
        let locked = LockedBlock::get(tx)?;
        let locked_block = locked.get_block(tx)?;

        // Liveness rules
        if self.justify().block_height() > locked_block.height() {
            return Ok(true);
        }

        // Safety rule
        if self.extends(tx, locked_block.id())? {
            return Ok(true);
        }

        info!(
            target: LOG_TARGET,
            "❌ Block {} does satisfy the liveness or safety rules of the safeNode predicate. Locked block {}",
            self,
            locked_block,
        );
        Ok(false)
    }
}

impl<TAddr> Display for Block<TAddr> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[{}, {}, {} command(s)]",
            self.height(),
            self.id(),
            self.commands().len()
        )
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

    pub const fn byte_size() -> usize {
        FixedHash::byte_size()
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
