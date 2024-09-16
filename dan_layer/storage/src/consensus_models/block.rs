//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{BTreeSet, HashSet},
    fmt::{Debug, Display, Formatter},
    hash::Hash,
    ops::{Deref, RangeInclusive},
};

use indexmap::IndexMap;
use log::*;
use serde::{Deserialize, Serialize};
use tari_common::configuration::Network;
use tari_common_types::types::{FixedHash, FixedHashSizeError, PublicKey};
use tari_crypto::{ristretto::RistrettoPublicKey, tari_utilities::epoch_time::EpochTime};
use tari_dan_common_types::{
    committee::CommitteeInfo,
    hashing,
    optional::Optional,
    serde_with,
    shard::Shard,
    Epoch,
    ExtraData,
    MaxSizeBytesError,
    NodeAddressable,
    NodeHeight,
    NumPreshards,
    ShardGroup,
    SubstateAddress,
};
use tari_transaction::TransactionId;
use time::PrimitiveDateTime;
#[cfg(feature = "ts")]
use ts_rs::TS;

use super::{
    BlockDiff,
    BlockPledge,
    ForeignProposal,
    ForeignProposalAtom,
    ForeignSendCounters,
    MintConfidentialOutputAtom,
    PendingShardStateTreeDiff,
    QuorumCertificate,
    SubstateChange,
    SubstateDestroyedProof,
    SubstatePledge,
    SubstateRecord,
    TransactionAtom,
    ValidatorSchnorrSignature,
};
use crate::{
    consensus_models::{
        Command,
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
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
};

const LOG_TARGET: &str = "tari::dan::storage::consensus_models::block";

#[derive(Debug, thiserror::Error)]
pub enum BlockError {
    #[error("Extra data size error: {0}")]
    ExtraDataSizeError(#[from] MaxSizeBytesError),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct Block {
    // Header
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    id: BlockId,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    network: Network,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    parent: BlockId,
    justify: QuorumCertificate,
    height: NodeHeight,
    epoch: Epoch,
    shard_group: ShardGroup,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    proposed_by: PublicKey,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    total_leader_fee: u64,
    // Body
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    merkle_root: FixedHash,
    // BTreeSet is used for the deterministic block hash, that is, transactions are always ordered by TransactionId.
    commands: BTreeSet<Command>,
    /// If the block is a dummy block. This is metadata and not sent over
    /// the wire or part of the block hash.
    is_dummy: bool,
    /// Flag that indicates that the block has been justified by a new high QC.
    is_justified: bool,
    /// Flag that indicates that the block has been committed.
    is_committed: bool,
    /// Counter for each foreign shard for reliable broadcast.
    foreign_indexes: IndexMap<Shard, u64>,
    /// Timestamp when was this stored.
    #[cfg_attr(feature = "ts", ts(type = "Array<number>| null"))]
    stored_at: Option<PrimitiveDateTime>,
    /// Signature of block by the proposer.
    #[cfg_attr(feature = "ts", ts(type = "{public_nonce : string, signature: string} | null"))]
    signature: Option<ValidatorSchnorrSignature>,
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    block_time: Option<u64>,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    timestamp: u64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    base_layer_block_height: u64,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    base_layer_block_hash: FixedHash,
    extra_data: Option<ExtraData>,
}

impl Block {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        network: Network,
        parent: BlockId,
        justify: QuorumCertificate,
        height: NodeHeight,
        epoch: Epoch,
        shard_group: ShardGroup,
        proposed_by: PublicKey,
        commands: BTreeSet<Command>,
        merkle_root: FixedHash,
        total_leader_fee: u64,
        sorted_foreign_indexes: IndexMap<Shard, u64>,
        signature: Option<ValidatorSchnorrSignature>,
        timestamp: u64,
        base_layer_block_height: u64,
        base_layer_block_hash: FixedHash,
        extra_data: Option<ExtraData>,
    ) -> Self {
        let mut block = Self {
            id: BlockId::zero(),
            network,
            parent,
            justify,
            height,
            epoch,
            shard_group,
            proposed_by,
            merkle_root,
            commands,
            total_leader_fee,
            is_dummy: false,
            is_justified: false,
            is_committed: false,
            foreign_indexes: sorted_foreign_indexes,
            stored_at: None,
            signature,
            block_time: None,
            timestamp,
            base_layer_block_height,
            base_layer_block_hash,
            extra_data,
        };
        block.id = block.calculate_hash().into();
        block
    }

    #[allow(clippy::too_many_arguments)]
    pub fn load(
        id: BlockId,
        network: Network,
        parent: BlockId,
        justify: QuorumCertificate,
        height: NodeHeight,
        epoch: Epoch,
        shard_group: ShardGroup,
        proposed_by: PublicKey,
        commands: BTreeSet<Command>,
        merkle_root: FixedHash,
        total_leader_fee: u64,
        is_dummy: bool,
        is_justified: bool,
        is_committed: bool,
        sorted_foreign_indexes: IndexMap<Shard, u64>,
        signature: Option<ValidatorSchnorrSignature>,
        created_at: PrimitiveDateTime,
        block_time: Option<u64>,
        timestamp: u64,
        base_layer_block_height: u64,
        base_layer_block_hash: FixedHash,
        extra_data: Option<ExtraData>,
    ) -> Self {
        Self {
            id,
            network,
            parent,
            justify,
            height,
            epoch,
            shard_group,
            proposed_by,
            merkle_root,
            commands,
            total_leader_fee,
            is_dummy,
            is_justified,
            is_committed,
            foreign_indexes: sorted_foreign_indexes,
            stored_at: Some(created_at),
            signature,
            block_time,
            timestamp,
            base_layer_block_height,
            base_layer_block_hash,
            extra_data,
        }
    }

    pub fn genesis(
        network: Network,
        epoch: Epoch,
        shard_group: ShardGroup,
        sidechain_id: Option<RistrettoPublicKey>,
    ) -> Result<Self, BlockError> {
        Ok(Self::new(
            network,
            BlockId::zero(),
            QuorumCertificate::genesis(epoch, shard_group),
            NodeHeight::zero(),
            epoch,
            shard_group,
            PublicKey::default(),
            Default::default(),
            // TODO: the merkle hash should be initialized to something committing to the previous state.
            FixedHash::zero(),
            0,
            IndexMap::new(),
            None,
            0,
            0,
            FixedHash::zero(),
            Self::extra_data_from_sidechain_id(sidechain_id)?,
        ))
    }

    /// This is the parent block for all genesis blocks. Its block ID is always zero.
    pub fn zero_block(
        network: Network,
        num_preshards: NumPreshards,
        sidechain_id: Option<RistrettoPublicKey>,
    ) -> Result<Self, BlockError> {
        Ok(Self {
            network,
            id: BlockId::zero(),
            parent: BlockId::zero(),
            justify: QuorumCertificate::genesis(Epoch::zero(), ShardGroup::all_shards(num_preshards)),
            height: NodeHeight::zero(),
            epoch: Epoch::zero(),
            shard_group: ShardGroup::all_shards(num_preshards),
            proposed_by: PublicKey::default(),
            merkle_root: FixedHash::zero(),
            commands: Default::default(),
            total_leader_fee: 0,
            is_dummy: false,
            is_justified: false,
            is_committed: true,
            foreign_indexes: IndexMap::new(),
            stored_at: None,
            signature: None,
            block_time: None,
            timestamp: EpochTime::now().as_u64(),
            base_layer_block_height: 0,
            base_layer_block_hash: FixedHash::zero(),
            extra_data: Self::extra_data_from_sidechain_id(sidechain_id)?,
        })
    }

    pub fn dummy_block(
        network: Network,
        parent: BlockId,
        proposed_by: PublicKey,
        height: NodeHeight,
        high_qc: QuorumCertificate,
        epoch: Epoch,
        shard_group: ShardGroup,
        parent_merkle_root: FixedHash,
        parent_timestamp: u64,
        parent_base_layer_block_height: u64,
        parent_base_layer_block_hash: FixedHash,
    ) -> Self {
        let mut block = Self {
            id: BlockId::zero(),
            network,
            parent,
            justify: high_qc,
            height,
            epoch,
            shard_group,
            proposed_by,
            merkle_root: parent_merkle_root,
            commands: BTreeSet::new(),
            total_leader_fee: 0,
            is_dummy: true,
            is_justified: false,
            is_committed: false,
            foreign_indexes: IndexMap::new(),
            stored_at: None,
            signature: None,
            block_time: None,
            timestamp: parent_timestamp,
            base_layer_block_height: parent_base_layer_block_height,
            base_layer_block_hash: parent_base_layer_block_hash,
            extra_data: None,
        };
        block.id = block.calculate_hash().into();
        block.is_justified = false;
        block
    }

    fn extra_data_from_sidechain_id(sidechain_id: Option<RistrettoPublicKey>) -> Result<Option<ExtraData>, BlockError> {
        let extra_data = sidechain_id
            .map(|id| ExtraData::new().insert_sidechain_id(id).cloned())
            .transpose()?;
        Ok(extra_data)
    }

    pub fn calculate_hash(&self) -> FixedHash {
        // Hash is created from the hash of the "body" and
        // then hashed with the parent, so that you can
        // create a merkle proof of a chain of blocks
        // ```pre
        // root
        // |\
        // |  block1
        // |\
        // |  block2
        // |
        // blockbody
        // ```

        let inner_hash = hashing::block_hasher()
            .chain(&self.network)
            // This allows us to exclude the justify and still validate the block
            .chain(self.justify.id())
            .chain(&self.height)
            .chain(&self.total_leader_fee)
            .chain(&self.epoch)
            .chain(&self.shard_group)
            .chain(&self.proposed_by)
            .chain(&self.merkle_root)
            .chain(&self.is_dummy)
            .chain(&self.commands)
            .chain(&self.foreign_indexes)
            .chain(&self.timestamp)
            .chain(&self.base_layer_block_height)
            .chain(&self.base_layer_block_hash)
            .chain(&self.extra_data)
            .result();

        hashing::block_hasher().chain(&self.parent).chain(&inner_hash).result()
    }
}

impl Block {
    pub fn is_genesis(&self) -> bool {
        self.height.is_zero()
    }

    pub fn is_epoch_end(&self) -> bool {
        self.commands.iter().any(|c| c.is_epoch_end())
    }

    pub fn all_transaction_ids(&self) -> impl Iterator<Item = &TransactionId> + '_ {
        self.commands.iter().filter_map(|d| d.transaction().map(|t| t.id()))
    }

    pub fn all_transaction_ids_in_committee<'a>(
        &'a self,
        committee_info: &'a CommitteeInfo,
    ) -> impl Iterator<Item = &TransactionId> + 'a {
        self.commands
            .iter()
            .filter_map(|cmd| cmd.transaction())
            .filter(|t| {
                t.evidence
                    .substate_addresses_iter()
                    .any(|addr| committee_info.includes_substate_address(addr))
            })
            .map(|t| t.id())
    }

    pub fn all_committing_transactions_ids(&self) -> impl Iterator<Item = &TransactionId> + '_ {
        self.commands.iter().filter_map(|d| d.committing()).map(|t| t.id())
    }

    pub fn all_finalising_transactions_ids(&self) -> impl Iterator<Item = &TransactionId> + '_ {
        self.commands.iter().filter_map(|d| d.finalising()).map(|t| t.id())
    }

    pub fn all_foreign_proposals(&self) -> impl Iterator<Item = &ForeignProposalAtom> + '_ {
        self.commands.iter().filter_map(|c| c.foreign_proposal())
    }

    pub fn all_confidential_output_mints(&self) -> impl Iterator<Item = &MintConfidentialOutputAtom> + '_ {
        self.commands.iter().filter_map(|c| c.mint_confidential_output())
    }

    pub fn all_some_prepare(&self) -> impl Iterator<Item = &TransactionAtom> + '_ {
        self.commands.iter().filter_map(|c| c.some_prepare())
    }

    pub fn command_count(&self) -> usize {
        self.commands.len()
    }

    pub fn as_locked_block(&self) -> LockedBlock {
        LockedBlock {
            height: self.height,
            block_id: self.id,
            epoch: self.epoch,
        }
    }

    pub fn as_last_executed(&self) -> LastExecuted {
        LastExecuted {
            height: self.height,
            block_id: self.id,
            epoch: self.epoch,
        }
    }

    pub fn as_last_voted(&self) -> LastVoted {
        LastVoted {
            height: self.height,
            block_id: self.id,
            epoch: self.epoch,
        }
    }

    pub fn as_leaf_block(&self) -> LeafBlock {
        LeafBlock {
            height: self.height,
            block_id: self.id,
            epoch: self.epoch,
        }
    }

    pub fn as_last_proposed(&self) -> LastProposed {
        LastProposed {
            height: self.height,
            block_id: self.id,
            epoch: self.epoch,
        }
    }

    pub fn id(&self) -> &BlockId {
        &self.id
    }

    pub fn network(&self) -> Network {
        self.network
    }

    pub fn parent(&self) -> &BlockId {
        &self.parent
    }

    pub fn justify(&self) -> &QuorumCertificate {
        &self.justify
    }

    pub fn into_justify(self) -> QuorumCertificate {
        self.justify
    }

    pub fn justifies_parent(&self) -> bool {
        *self.justify.block_id() == self.parent
    }

    pub fn height(&self) -> NodeHeight {
        self.height
    }

    pub fn is_zero(&self) -> bool {
        self.id.is_zero()
    }

    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    pub fn shard_group(&self) -> ShardGroup {
        self.shard_group
    }

    pub fn total_leader_fee(&self) -> u64 {
        self.total_leader_fee
    }

    pub fn total_transaction_fee(&self) -> u64 {
        self.commands
            .iter()
            .filter_map(|c| c.committing())
            .map(|atom| atom.transaction_fee)
            .sum()
    }

    pub fn proposed_by(&self) -> &PublicKey {
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

    pub fn is_justified(&self) -> bool {
        self.is_justified
    }

    pub fn is_committed(&self) -> bool {
        self.is_committed
    }

    pub fn get_foreign_counter(&self, bucket: &Shard) -> Option<u64> {
        self.foreign_indexes.get(bucket).copied()
    }

    pub fn foreign_indexes(&self) -> &IndexMap<Shard, u64> {
        &self.foreign_indexes
    }

    pub fn block_time(&self) -> Option<u64> {
        self.block_time
    }

    pub fn timestamp(&self) -> u64 {
        self.timestamp
    }

    pub fn signature(&self) -> Option<&ValidatorSchnorrSignature> {
        self.signature.as_ref()
    }

    pub fn set_signature(&mut self, signature: ValidatorSchnorrSignature) {
        self.signature = Some(signature);
    }

    pub fn is_proposed_by_addr<A: NodeAddressable + PartialEq<A>>(&self, address: &A) -> Option<bool> {
        Some(A::try_from_public_key(&self.proposed_by)? == *address)
    }

    pub fn base_layer_block_height(&self) -> u64 {
        self.base_layer_block_height
    }

    pub fn base_layer_block_hash(&self) -> &FixedHash {
        &self.base_layer_block_hash
    }

    pub fn extra_data(&self) -> Option<&ExtraData> {
        self.extra_data.as_ref()
    }
}

impl Block {
    pub fn get<TTx: StateStoreReadTransaction + ?Sized>(tx: &TTx, id: &BlockId) -> Result<Self, StorageError> {
        tx.blocks_get(id)
    }

    /// Returns all blocks from and excluding the start block (lower height) to the end block (inclusive)
    pub fn get_all_blocks_between<TTx: StateStoreReadTransaction>(
        tx: &TTx,
        epoch: Epoch,
        shard_group: ShardGroup,
        start_block_id: &BlockId,
        end_block_id: &BlockId,
        include_dummy_blocks: bool,
    ) -> Result<Vec<Self>, StorageError> {
        tx.blocks_get_all_between(epoch, shard_group, start_block_id, end_block_id, include_dummy_blocks)
    }

    pub fn get_last_n_in_epoch<TTx: StateStoreReadTransaction + ?Sized>(
        tx: &TTx,
        n: usize,
        epoch: Epoch,
    ) -> Result<Vec<Self>, StorageError> {
        tx.blocks_get_last_n_in_epoch(n, epoch)
    }

    pub fn exists<TTx: StateStoreReadTransaction + ?Sized>(&self, tx: &TTx) -> Result<bool, StorageError> {
        Self::record_exists(tx, self.id())
    }

    pub fn parent_exists<TTx: StateStoreReadTransaction + ?Sized>(&self, tx: &TTx) -> Result<bool, StorageError> {
        Self::record_exists(tx, self.parent())
    }

    pub fn has_been_processed<TTx: StateStoreReadTransaction + ?Sized>(
        tx: &TTx,
        block_id: &BlockId,
    ) -> Result<bool, StorageError> {
        // TODO: consider optimising
        let is_justified = Self::get(tx, block_id)
            .optional()?
            .map(|b| b.is_justified())
            .unwrap_or(false);
        Ok(is_justified)
    }

    pub fn record_exists<TTx: StateStoreReadTransaction + ?Sized>(
        tx: &TTx,
        block_id: &BlockId,
    ) -> Result<bool, StorageError> {
        tx.blocks_exists(block_id)
    }

    pub fn insert<TTx: StateStoreWriteTransaction + ?Sized>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        tx.blocks_insert(self)
    }

    // pub fn get_paginated<TTx: StateStoreReadTransaction>(
    //     tx: &mut TTx,
    //     limit: u64,
    //     offset: u64,
    //     ordering: Option<Ordering>,
    // ) -> Result<Vec<Self>, StorageError> {
    //     tx.blocks_get_paginated(limit, offset, ordering)
    // }

    pub fn get_count<TTx: StateStoreReadTransaction>(tx: &TTx) -> Result<i64, StorageError> {
        tx.blocks_get_count()
    }

    /// Inserts the block if it doesnt exist. Returns true if the block was saved and did not exist previously,
    /// otherwise false.
    pub fn save<TTx>(&self, tx: &mut TTx) -> Result<bool, StorageError>
    where
        TTx: StateStoreWriteTransaction + Deref,
        TTx::Target: StateStoreReadTransaction,
    {
        let exists = self.exists(&**tx)?;
        if exists {
            return Ok(false);
        }
        self.insert(tx)?;
        Ok(true)
    }

    pub fn remove_parallel_chains<TTx>(&self, tx: &mut TTx) -> Result<(), StorageError>
    where
        TTx: StateStoreWriteTransaction + Deref,
        TTx::Target: StateStoreReadTransaction,
    {
        let other_blocks = tx.blocks_get_all_ids_by_height(self.epoch(), self.height())?;
        for block_id in other_blocks {
            if block_id == *self.id() {
                continue;
            }
            delete_block_and_children(tx, &block_id)?;
        }
        Ok(())
    }

    pub fn remove_diff<TTx: StateStoreWriteTransaction>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        tx.block_diffs_remove(self.id())
    }

    pub fn remove_pending_tree_diff<TTx: StateStoreWriteTransaction>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        tx.pending_state_tree_diffs_remove_by_block(self.id())
    }

    pub fn remove_pending_tree_diff_and_return<TTx: StateStoreWriteTransaction>(
        &self,
        tx: &mut TTx,
    ) -> Result<IndexMap<Shard, Vec<PendingShardStateTreeDiff>>, StorageError> {
        tx.pending_state_tree_diffs_remove_and_return_by_block(self.id())
    }

    pub fn delete<TTx: StateStoreWriteTransaction>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        Self::delete_record(tx, self.id())
    }

    pub fn delete_record<TTx: StateStoreWriteTransaction>(
        tx: &mut TTx,
        block_id: &BlockId,
    ) -> Result<(), StorageError> {
        tx.blocks_delete(block_id)
    }

    pub fn commit_diff<TTx: StateStoreWriteTransaction>(
        &self,
        tx: &mut TTx,
        block_diff: BlockDiff,
    ) -> Result<(), StorageError> {
        if block_diff.block_id() != self.id() {
            return Err(StorageError::QueryError {
                reason: format!(
                    "[commit_diff] Block ID mismatch. Expected: {}, got: {}",
                    self.id(),
                    block_diff.block_id()
                ),
            });
        }

        if self.is_dummy() && !block_diff.is_empty() {
            return Err(StorageError::QueryError {
                reason: format!(
                    "[commit_diff] Dummy block cannot have any substate changes. Block ID: {}",
                    self.id()
                ),
            });
        }

        if !self.is_dummy() {
            block_diff.remove(tx)?;
        }

        for change in block_diff.into_changes() {
            match change {
                SubstateChange::Up {
                    id,
                    shard,
                    transaction_id,
                    substate,
                } => {
                    SubstateRecord::new(
                        id.substate_id,
                        id.version,
                        substate.into_substate_value(),
                        shard,
                        self.epoch(),
                        self.height(),
                        *self.id(),
                        transaction_id,
                        *self.justify().id(),
                    )
                    .create(tx)?;
                },
                SubstateChange::Down {
                    id,
                    transaction_id,
                    shard,
                } => {
                    SubstateRecord::destroy(
                        tx,
                        id,
                        shard,
                        self.epoch(),
                        self.height(),
                        self.justify().id(),
                        &transaction_id,
                    )?;
                },
            }
        }

        tx.blocks_set_flags(self.id(), Some(true), None)
    }

    pub fn get_diff<TTx: StateStoreReadTransaction>(&self, tx: &TTx) -> Result<BlockDiff, StorageError> {
        tx.block_diffs_get(self.id())
    }

    pub fn set_as_justified<TTx: StateStoreWriteTransaction>(&mut self, tx: &mut TTx) -> Result<(), StorageError> {
        self.is_justified = true;
        tx.blocks_set_flags(self.id(), None, Some(true))
    }

    pub fn find_involved_shards<TTx: StateStoreReadTransaction>(
        &self,
        tx: &TTx,
    ) -> Result<HashSet<SubstateAddress>, StorageError> {
        tx.transactions_fetch_involved_shards(self.all_transaction_ids().copied().collect())
    }

    pub fn max_height<TTx: StateStoreReadTransaction>(tx: &TTx) -> Result<NodeHeight, StorageError> {
        tx.blocks_max_height()
    }

    pub fn extends<TTx: StateStoreReadTransaction>(&self, tx: &TTx, ancestor: &BlockId) -> Result<bool, StorageError> {
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

    pub fn get_parent<TTx: StateStoreReadTransaction + ?Sized>(&self, tx: &TTx) -> Result<Block, StorageError> {
        if self.id.is_zero() && self.parent.is_zero() {
            return Err(StorageError::NotFound {
                item: "Block parent".to_string(),
                key: self.parent.to_string(),
            });
        }

        Block::get(tx, &self.parent)
    }

    pub fn get_parent_chain<TTx: StateStoreReadTransaction>(
        &self,
        tx: &TTx,
        limit: usize,
    ) -> Result<Vec<Block>, StorageError> {
        tx.blocks_get_parent_chain(self.id(), limit)
    }

    pub fn get_votes<TTx: StateStoreReadTransaction>(&self, tx: &TTx) -> Result<Vec<Vote>, StorageError> {
        Vote::get_for_block(tx, &self.id)
    }

    pub fn get_child_block_ids<TTx: StateStoreReadTransaction>(&self, tx: &TTx) -> Result<Vec<BlockId>, StorageError> {
        tx.blocks_get_ids_by_parent(self.id())
    }

    pub fn get_total_due_for_epoch<TTx: StateStoreReadTransaction>(
        tx: &TTx,
        epoch: Epoch,
        validator_public_key: &PublicKey,
    ) -> Result<u64, StorageError> {
        tx.blocks_get_total_leader_fee_for_epoch(epoch, validator_public_key)
    }

    pub fn get_any_with_epoch_range_for_validator<TTx: StateStoreReadTransaction>(
        tx: &TTx,
        range: RangeInclusive<Epoch>,
        validator_public_key: Option<&PublicKey>,
    ) -> Result<Vec<Self>, StorageError> {
        tx.blocks_get_any_with_epoch_range(range, validator_public_key)
    }

    pub fn get_transactions<TTx: StateStoreReadTransaction>(
        &self,
        tx: &TTx,
    ) -> Result<Vec<TransactionRecord>, StorageError> {
        let tx_ids = self.commands().iter().filter_map(|t| t.transaction().map(|t| t.id()));
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

    pub fn get_substate_updates<TTx: StateStoreReadTransaction>(
        &self,
        tx: &TTx,
    ) -> Result<Vec<SubstateUpdate>, StorageError> {
        let committed = self
            .commands()
            .iter()
            .filter_map(|c| c.committing())
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
                            // created_qc: substate.get_created_quorum_certificate(tx)?,
                            substate: substate.into(),
                        }));
                    } else {
                        updates.push(SubstateUpdate::Destroy(SubstateDestroyedProof {
                            substate_id: substate.substate_id.clone(),
                            version: substate.version,
                            // justify: QuorumCertificate::get(tx, &destroyed.justify)?,
                            destroyed_by_transaction: destroyed.by_transaction,
                        }));
                    }
                } else {
                    updates.push(SubstateUpdate::Create(SubstateCreatedProof {
                        // created_qc: substate.get_created_quorum_certificate(tx)?,
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
        mut on_lock_block: TFnOnLock,
        mut on_commit: TFnOnCommit,
    ) -> Result<Self, E>
    where
        TTx: StateStoreWriteTransaction + Deref + ?Sized,
        TTx::Target: StateStoreReadTransaction,
        TFnOnLock: FnMut(&mut TTx, &LockedBlock, &Block, &QuorumCertificate) -> Result<(), E>,
        TFnOnCommit: FnMut(&mut TTx, &LastExecuted, &Block) -> Result<(), E>,
        E: From<StorageError>,
    {
        self.justify().update_high_qc(tx)?;

        // b'' <- b*.justify.node i.e. the (possibly new) justified block
        let justified_node = self.justify().get_block(&**tx)?;

        // b' <- b''.justify.node
        let prepared_node = justified_node.justify().get_block(&**tx)?;

        if prepared_node.is_genesis() {
            return Ok(justified_node);
        }

        let current_locked = LockedBlock::get(&**tx, self.epoch)?;
        if prepared_node.height() > current_locked.height {
            on_locked_block_recurse(
                tx,
                &current_locked,
                &prepared_node,
                justified_node.justify(),
                &mut on_lock_block,
            )?;
            prepared_node.as_locked_block().set(tx)?;
        }

        // b <- b'.justify.node
        let commit_node = prepared_node.justify().block_id();
        if justified_node.parent() == prepared_node.id() && prepared_node.parent() == commit_node {
            debug!(
                target: LOG_TARGET,
                "✅ Node {} {} forms a 3-chain b'' = {}, b' = {}, b = {}",
                self.height(),
                self.id(),
                justified_node.id(),
                prepared_node.id(),
                commit_node,
            );

            // Commit prepare_node (b)
            if commit_node.is_zero() {
                return Ok(justified_node);
            }
            let prepare_node = Block::get(&**tx, commit_node)?;
            let last_executed = LastExecuted::get(&**tx)?;
            on_commit_block_recurse(tx, &last_executed, &prepare_node, &mut on_commit)?;
            prepare_node.as_last_executed().set(tx)?;
        } else {
            debug!(
                target: LOG_TARGET,
                "Node {} {} DOES NOT form a 3-chain b'' = {}, b' = {}, b = {}, b* = {}",
                self.height(),
                self.id(),
                justified_node.id(),
                prepared_node.id(),
                commit_node,
                self.id()
            );
        }

        Ok(justified_node)
    }

    /// safeNode predicate (https://arxiv.org/pdf/1803.05069v6.pdf)
    ///
    /// The safeNode predicate is a core ingredient of the protocol. It examines a proposal message
    /// m carrying a QC justification m.justify, and determines whether m.node is safe to accept. The safety rule to
    /// accept a proposal is the branch of m.node extends from the currently locked node lockedQC.node. On the other
    /// hand, the liveness rule is the replica will accept m if m.justify has a higher view than the current
    /// lockedQC. The predicate is true as long as either one of two rules holds.
    pub fn is_safe<TTx: StateStoreReadTransaction>(&self, tx: &TTx) -> Result<bool, StorageError> {
        let locked = LockedBlock::get(tx, self.epoch())?;
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

    pub fn save_foreign_send_counters<TTx>(&self, tx: &mut TTx) -> Result<(), StorageError>
    where
        TTx: StateStoreWriteTransaction + Deref + ?Sized,
        TTx::Target: StateStoreReadTransaction,
    {
        let mut counters = ForeignSendCounters::get_or_default(&**tx, self.justify().block_id())?;
        // Add counters for this block and carry over the counters from the justify block, if any
        for shard in self.foreign_indexes.keys() {
            counters.increment_counter(*shard);
        }
        if !counters.is_empty() {
            counters.set(tx, self.id())?;
        }
        Ok(())
    }

    pub fn get_block_pledge<TTx: StateStoreReadTransaction>(&self, tx: &TTx) -> Result<BlockPledge, StorageError> {
        let mut pledges = BlockPledge::new();
        for atom in self
            .commands()
            .iter()
            .filter_map(|cmd| cmd.local_prepare().or_else(|| cmd.local_accept()))
        {
            // No pledges for aborted transactions
            if atom.decision.is_abort() {
                continue;
            }

            let evidence = atom
                .evidence
                .get(&self.shard_group)
                .ok_or_else(|| StorageError::DataInconsistency {
                    details: format!(
                        "invariant get_block_pledge: Local evidence for atom {} in block {} is missing",
                        atom.id, self.id
                    ),
                })?;

            // TODO(perf): O(n) queries
            let locked_values = tx.substate_locks_get_locked_substates_for_transaction(&atom.id)?;

            // CASE: We're retrieving pledges for the LocalPrepared and LocalAccept commands. If all other pledges were
            // provided already, we may have progressed to AllPrepared, executed the transaction and locked
            // local outputs. However, these are not provided in the original LocalPrepare evidence for this
            // atom, so we need to exclude them.
            let locks = locked_values
                .into_iter()
                .filter(|lock| evidence.contains(&lock.to_substate_address()));

            pledges.reserve(locks.clone().count());
            for locked_value in locks {
                let lock_intent = locked_value.to_substate_lock_intent();
                let pledge = SubstatePledge::try_create(lock_intent.clone(), locked_value.value).ok_or_else(|| {
                    StorageError::DataInconsistency {
                        details: format!("SubstatePledge ({}) is not valid", lock_intent),
                    }
                })?;
                pledges.add_substate_pledge(*locked_value.lock.transaction_id(), pledge);
            }
        }
        Ok(pledges)
    }

    pub fn get_foreign_proposals<TTx: StateStoreReadTransaction>(
        &self,
        tx: &TTx,
    ) -> Result<Vec<ForeignProposal>, StorageError> {
        ForeignProposal::get_any(tx, self.all_foreign_proposals().map(|p| &p.block_id))
    }
}

impl Display for Block {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[{}, {}, {}, {} cmd(s), {}]",
            self.height(),
            self.epoch(),
            self.shard_group(),
            self.commands().len(),
            self.id(),
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

    pub const fn zero() -> Self {
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

    pub fn is_zero(&self) -> bool {
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
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Display::fmt(&self.0, f)
    }
}

fn on_locked_block_recurse<TTx, F, E>(
    tx: &mut TTx,
    locked: &LockedBlock,
    block: &Block,
    justify_qc: &QuorumCertificate,
    callback: &mut F,
) -> Result<(), E>
where
    TTx: StateStoreWriteTransaction + Deref + ?Sized,
    TTx::Target: StateStoreReadTransaction,
    E: From<StorageError>,
    F: FnMut(&mut TTx, &LockedBlock, &Block, &QuorumCertificate) -> Result<(), E>,
{
    if locked.height < block.height() {
        let parent = block.get_parent(&**tx)?;
        on_locked_block_recurse(tx, locked, &parent, block.justify(), callback)?;
        callback(tx, locked, block, justify_qc)?;
    }
    Ok(())
}

fn on_commit_block_recurse<TTx, F, E>(
    tx: &mut TTx,
    last_executed: &LastExecuted,
    block: &Block,
    callback: &mut F,
) -> Result<(), E>
where
    TTx: StateStoreWriteTransaction + Deref + ?Sized,
    TTx::Target: StateStoreReadTransaction,
    E: From<StorageError>,
    F: FnMut(&mut TTx, &LastExecuted, &Block) -> Result<(), E>,
{
    if last_executed.height < block.height() {
        let parent = block.get_parent(&**tx)?;
        // Recurse to "catch up" any parent parent blocks we may not have executed
        on_commit_block_recurse(tx, last_executed, &parent, callback)?;
        callback(tx, last_executed, block)?;
    }
    Ok(())
}

/// Deletes everything related to a block and any children
fn delete_block_and_children<TTx>(tx: &mut TTx, block_id: &BlockId) -> Result<(), StorageError>
where
    TTx: StateStoreWriteTransaction + Deref,
    TTx::Target: StateStoreReadTransaction,
{
    let children = tx.blocks_get_ids_by_parent(block_id)?;
    for child in children {
        delete_block_and_children(tx, &child)?;
    }
    tx.block_diffs_remove(block_id).optional()?;
    tx.pending_state_tree_diffs_remove_by_block(block_id).optional()?;
    tx.substate_locks_remove_any_by_block_id(block_id)?;
    tx.transaction_pool_state_updates_remove_any_by_block_id(block_id)?;
    tx.transaction_executions_remove_any_by_block_id(block_id)?;
    tx.foreign_proposals_clear_proposed_in(block_id)?;
    tx.burnt_utxos_clear_proposed_block(block_id)?;

    Block::delete_record(tx, block_id)?;

    Ok(())
}
