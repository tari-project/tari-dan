//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{BTreeSet, HashMap},
    fmt::{Debug, Display, Formatter},
    iter,
    ops::Deref,
};

use indexmap::IndexMap;
use log::*;
use minicbor::{CborLen, Decode, Encode};
use serde::{Deserialize, Serialize};
use tari_common_types::types::FixedHash;
use tari_consensus_types::{
    BlockId,
    HighestSeenBlock,
    LastExecuted,
    LastProposed,
    LastVoted,
    LeafBlock,
    LockedBlock,
    PcId,
    ProposalCertificate,
    ShardGroupAccumulatedData,
    TimeoutCertificate,
};
use tari_ootle_common_types::{
    Epoch,
    ExtraData,
    ExtraFieldKey,
    NodeHeight,
    NumPreshards,
    ProtocolVersion,
    ShardGroup,
    VersionedSubstateId,
    VersionedSubstateIdRef,
    committee::CommitteeInfo,
    displayable::Displayable,
    optional::Optional,
    shard::Shard,
};
use tari_ootle_transaction::{Network, TransactionId};
use tari_state_tree::{SparseMerkleProofExt, StateTreeError, TreeHash, Version, compute_proof_for_hashes};
use tari_template_lib_types::{
    TransactionReceiptAddress,
    crypto::{RistrettoPublicKeyBytes, SchnorrSignatureBytes},
};
use time::PrimitiveDateTime;

use super::{
    BlockDiff,
    BlockPledge,
    BookkeepingModel,
    ForeignProposalAtom,
    ForeignProposalRecord,
    LockedEpoch,
    PendingShardStateTreeDiff,
    SubstateDestroy,
    SubstateRecord,
    TransactionAtom,
    ValidatorStatsUpdate,
};
use crate::{
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
    consensus_models::{
        Command,
        SubstateCreate,
        SubstateUpdateProof,
        TransactionRecord,
        block_header::BlockHeader,
        substate_update_batch::SubstateUpdateBatch,
    },
};

const LOG_TARGET: &str = "tari::ootle::storage::consensus_models::block";

#[derive(Debug, thiserror::Error)]
pub enum BlockError {
    #[error("Error computing command merkle hash: {0}")]
    StateTreeError(#[from] StateTreeError),
    #[error("Invalid shard state versions: {details}")]
    InvalidShardStateVersions { details: String },
    #[error("Merke proof generation command index out of bounds: {index}/{len}")]
    MerkleProofGenerationCommandIndexOutOfBounds { index: usize, len: usize },
}

#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode, CborLen)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Block {
    #[n(0)]
    header: BlockHeader,
    /// Collection of signatures that justify a previous block and potentially a change to the next higher view.
    #[n(1)]
    justify: ProposalCertificate,
    /// Commands in the block. These are in canonical order to ensure a deterministic block hash.
    #[n(2)]
    commands: BTreeSet<Command>,
    /// The block's justification for a view timeout. This is only relevant if it is for a higher view height than the
    /// ProposalCertificate.
    #[n(3)]
    timeout_certificate: Option<TimeoutCertificate>,
    // Metadata - not included in the block hash
    /// The QC that justified this block
    #[cfg_attr(feature = "ts", ts(type = "string | null"))]
    #[n(4)]
    justify_qc_id: Option<PcId>,
    /// The QC that caused this block to be committed
    #[cfg_attr(feature = "ts", ts(type = "string | null"))]
    #[n(5)]
    commit_qc_id: Option<PcId>,
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    #[n(6)]
    block_time: Option<u64>,
    /// Timestamp when was this stored.
    #[cfg_attr(feature = "ts", ts(type = "string | null"))]
    // PrimitiveDateTime is foreign (time crate) — bridge through serde.
    #[n(7)]
    #[cbor(with = "tari_bor::adapters::serde_bridge")]
    stored_at: Option<PrimitiveDateTime>,
}

impl Block {
    /// Creates a new block from the provided params. Returns an error if the command merkle root fails to construct.
    /// This is infallible for empty commands.
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        network: Network,
        protocol_version: ProtocolVersion,
        parent: BlockId,
        justify: ProposalCertificate,
        high_tc: Option<TimeoutCertificate>,
        height: NodeHeight,
        epoch: Epoch,
        shard_group: ShardGroup,
        proposed_by: RistrettoPublicKeyBytes,
        commands: BTreeSet<Command>,
        state_merkle_root: FixedHash,
        total_leader_fee: u64,
        signature: SchnorrSignatureBytes,
        timestamp: u64,
        epoch_hash: FixedHash,
        accumulated_data: ShardGroupAccumulatedData,
        extra_data: ExtraData,
    ) -> Result<Self, BlockError> {
        let header = BlockHeader::create(
            network,
            protocol_version,
            parent,
            justify.calculate_id(),
            height,
            epoch,
            shard_group,
            proposed_by,
            state_merkle_root,
            &commands,
            total_leader_fee,
            signature,
            timestamp,
            epoch_hash,
            accumulated_data,
            extra_data,
        )?;
        Ok(Self::new(header, justify, commands, high_tc))
    }

    pub fn new(
        header: BlockHeader,
        justify: ProposalCertificate,
        commands: BTreeSet<Command>,
        timeout_certificate: Option<TimeoutCertificate>,
    ) -> Self {
        Self {
            header,
            justify,
            commands,
            timeout_certificate,
            justify_qc_id: None,
            commit_qc_id: None,
            block_time: None,
            stored_at: None,
        }
    }

    pub fn genesis(
        network: Network,
        protocol_version: ProtocolVersion,
        epoch: Epoch,
        epoch_hash: FixedHash,
        shard_group: ShardGroup,
        state_merkle_root: FixedHash,
        sidechain_id: Option<RistrettoPublicKeyBytes>,
    ) -> Self {
        let mut extra_data = ExtraData::new();
        if let Some(sidechain_id) = sidechain_id {
            extra_data.insert(
                ExtraFieldKey::SidechainId,
                sidechain_id
                    .as_bytes()
                    .try_into()
                    .expect("RistrettoPublicKey is 32 bytes"),
            );
        }
        let justify = ProposalCertificate::genesis(epoch, shard_group);
        let header = BlockHeader::genesis(
            network,
            protocol_version,
            justify.calculate_id(),
            epoch,
            shard_group,
            state_merkle_root,
            epoch_hash,
            // Each genesis block starts with zero accumulated data
            ShardGroupAccumulatedData::default(),
            extra_data,
        );
        Self::new(header, justify, BTreeSet::new(), None)
    }

    /// This is the parent block for all genesis blocks. Its block ID is always zero.
    pub fn zero_block(network: Network, num_preshards: NumPreshards) -> Self {
        let qc = ProposalCertificate::genesis(Epoch::zero(), ShardGroup::all_shards(num_preshards));
        Self {
            header: BlockHeader::zero_block(network, num_preshards),
            commit_qc_id: Some(qc.calculate_id()),
            justify: qc,
            timeout_certificate: None,
            commands: Default::default(),
            justify_qc_id: None,
            stored_at: None,
            block_time: None,
        }
    }

    pub fn calculate_id(&self) -> BlockId {
        self.header.calculate_id()
    }

    pub fn header(&self) -> &BlockHeader {
        &self.header
    }

    pub fn is_genesis(&self) -> bool {
        self.header().is_genesis()
    }

    pub fn is_epoch_end(&self) -> bool {
        // EOE block only has a single EndEpoch command
        self.commands.first().is_some_and(|c| c.is_epoch_end())
    }

    pub fn all_transaction_ids(&self) -> impl Iterator<Item = &TransactionId> + '_ {
        self.commands.iter().filter_map(|d| d.transaction().map(|t| t.id()))
    }

    pub fn all_transaction_ids_in_committee<'a>(
        &'a self,
        committee_info: &'a CommitteeInfo,
    ) -> impl Iterator<Item = &'a TransactionId> + Clone + 'a {
        self.commands
            .iter()
            .filter_map(|cmd| cmd.transaction())
            .filter(|t| t.evidence.has_and_not_empty(&committee_info.shard_group()))
            .map(|t| t.id())
    }

    pub fn all_committing_transactions_ids(&self) -> impl Iterator<Item = &TransactionId> + '_ {
        self.commands.iter().filter_map(|d| d.committing()).map(|t| t.id())
    }

    pub fn all_finalising_transactions_ids(&self) -> impl Iterator<Item = &TransactionId> + '_ {
        self.commands.iter().filter_map(|d| d.finalising()).map(|t| t.id())
    }

    pub fn all_aborting_transaction_ids(&self) -> impl Iterator<Item = &TransactionId> + '_ {
        self.commands.iter().filter_map(|d| d.aborting()).map(|t| t.id())
    }

    pub fn all_foreign_proposals(&self) -> impl Iterator<Item = &ForeignProposalAtom> + '_ {
        self.commands.iter().filter_map(|c| c.foreign_proposal())
    }

    pub fn all_local_accept(&self) -> impl Iterator<Item = &TransactionAtom> + '_ {
        self.commands.iter().filter_map(|c| c.local_accept())
    }

    pub fn command_count(&self) -> usize {
        self.commands.len()
    }

    pub fn as_locked(&self) -> LockedBlock {
        self.header().as_locked()
    }

    pub fn as_last_executed(&self) -> LastExecuted {
        self.header().as_last_executed()
    }

    pub fn as_last_voted(&self) -> LastVoted {
        self.header().as_last_voted()
    }

    pub fn as_leaf(&self) -> LeafBlock {
        self.header().as_leaf()
    }

    pub fn as_highest_seen(&self) -> HighestSeenBlock {
        HighestSeenBlock {
            height: self.header.height(),
            block_id: *self.id(),
            epoch: self.header.epoch(),
            shard_group: self.header.shard_group(),
        }
    }

    pub fn as_last_proposed(&self) -> LastProposed {
        LastProposed {
            height: self.header.height(),
            block_id: *self.id(),
            epoch: self.header.epoch(),
            shard_group: self.header.shard_group(),
        }
    }

    pub fn id(&self) -> &BlockId {
        self.header.id()
    }

    pub fn network(&self) -> Network {
        self.header.network()
    }

    pub fn parent(&self) -> &BlockId {
        self.header.parent()
    }

    pub fn justify(&self) -> &ProposalCertificate {
        &self.justify
    }

    pub fn max_certificate_height(&self) -> NodeHeight {
        self.justify.height().max(
            self.timeout_certificate
                .as_ref()
                .map(|tc| tc.height())
                .unwrap_or_else(NodeHeight::zero),
        )
    }

    pub fn into_justify(self) -> ProposalCertificate {
        self.justify
    }

    pub fn justifies_parent(&self) -> bool {
        self.justify.calculate_block_id() == *self.parent()
    }

    pub fn height(&self) -> NodeHeight {
        self.header.height()
    }

    pub fn epoch(&self) -> Epoch {
        self.header.epoch()
    }

    pub fn epoch_hash(&self) -> &FixedHash {
        self.header.epoch_hash()
    }

    pub fn to_locked_epoch(&self) -> LockedEpoch {
        LockedEpoch::new(self.epoch(), self.epoch_hash().into_array().into())
    }

    pub fn shard_group(&self) -> ShardGroup {
        self.header.shard_group()
    }

    pub fn total_leader_fee(&self) -> u64 {
        self.header.total_leader_fee()
    }

    pub fn calculate_total_transaction_fee(&self) -> u64 {
        self.commands
            .iter()
            .filter_map(|c| c.committing())
            .map(|atom| atom.transaction_fee)
            .sum()
    }

    pub fn proposed_by(&self) -> &RistrettoPublicKeyBytes {
        self.header.proposed_by()
    }

    pub fn state_merkle_root(&self) -> &FixedHash {
        self.header.state_merkle_root()
    }

    pub fn command_merkle_root(&self) -> &FixedHash {
        self.header.command_merkle_root()
    }

    pub fn commands(&self) -> &BTreeSet<Command> {
        &self.commands
    }

    pub fn into_commands(self) -> BTreeSet<Command> {
        self.commands
    }

    pub fn is_dummy(&self) -> bool {
        self.header.is_dummy()
    }

    pub fn timeout_certificate(&self) -> Option<&TimeoutCertificate> {
        self.timeout_certificate.as_ref()
    }

    /// Whether some branch has justified this block.
    ///
    /// This is stamped by a block carrying a QC over this one, before that block is voted on, and stays
    /// stamped across the whole block store even where that branch is later abandoned. Work whose result
    /// belongs to one branch must therefore key off that branch rather than off this flag.
    pub fn has_justify_qc(&self) -> bool {
        self.justify_qc_id.is_some()
    }

    /// The id of a QC over this block, as last recorded by [`Self::add_justify_qc`]. Carries the same
    /// whole-store scope as [`Self::has_justify_qc`].
    pub fn justify_qc_id(&self) -> Option<PcId> {
        self.justify_qc_id
    }

    pub fn is_committed(&self) -> bool {
        self.commit_qc_id.is_some()
    }

    pub fn block_time(&self) -> Option<u64> {
        self.block_time
    }

    pub fn timestamp(&self) -> u64 {
        self.header.timestamp()
    }

    pub fn signature(&self) -> Option<&SchnorrSignatureBytes> {
        self.header.signature()
    }

    pub fn extra_data(&self) -> &ExtraData {
        self.header.extra_data()
    }

    pub fn compute_command_inclusion_proof(&self, command_index: usize) -> Result<SparseMerkleProofExt, BlockError> {
        let hashes = self.commands.iter().map(|cmd| TreeHash::from(cmd.hash().into_array()));
        let hash =
            hashes
                .clone()
                .nth(command_index)
                .ok_or(BlockError::MerkleProofGenerationCommandIndexOutOfBounds {
                    index: command_index,
                    len: self.commands.len(),
                })?;
        let (value, proof) = compute_proof_for_hashes(hashes, hash)?;
        value.expect(
            "Value not found in proof. This is a bug because the hash is taken from commands that generate the tree",
        );
        Ok(proof)
    }

    pub fn set_justify_qc(&mut self, justify_qc_id: PcId) {
        self.justify_qc_id = Some(justify_qc_id);
    }

    pub fn set_commit_qc(&mut self, commit_qc_id: PcId) {
        self.commit_qc_id = Some(commit_qc_id);
    }

    pub fn commit_qc_id(&self) -> Option<&PcId> {
        self.commit_qc_id.as_ref()
    }
}

impl Block {
    pub fn get<TTx: StateStoreReadTransaction>(tx: &TTx, id: &BlockId) -> Result<Self, StorageError> {
        tx.blocks_get(id)
    }

    pub fn get_ids_by_epoch_and_height<TTx: StateStoreReadTransaction>(
        tx: &TTx,
        epoch: Epoch,
        height: NodeHeight,
    ) -> Result<Vec<BlockId>, StorageError> {
        tx.blocks_get_all_ids_by_height(epoch, height)
    }

    /// Returns the block justified by the given proposal certificate.
    ///
    /// When the QC justifies the zero block, `calculate_block_id()` returns `BlockId::zero()` which
    /// is the global zero block (epoch 0, all-zero fields). However, the epoch-specific genesis block
    /// has non-trivial fields (state_merkle_root, epoch_hash, etc.) that are needed for deterministic
    /// dummy block computation. This method correctly resolves to the epoch genesis in that case.
    pub fn get_justified_block<TTx: StateStoreReadTransaction>(
        tx: &TTx,
        justify: &ProposalCertificate,
        epoch: Epoch,
    ) -> Result<Self, StorageError> {
        if justify.justifies_zero_block() {
            Self::get_genesis_for_epoch(tx, epoch)
        } else {
            Self::get(tx, &justify.calculate_block_id())
        }
    }

    pub fn get_genesis_for_epoch<TTx: StateStoreReadTransaction>(tx: &TTx, epoch: Epoch) -> Result<Self, StorageError> {
        let ids = Self::get_ids_by_epoch_and_height(tx, epoch, NodeHeight::zero())?;
        if ids.is_empty() {
            return Err(StorageError::DataInconsistency {
                details: format!("No genesis block found for epoch {}", epoch),
            });
        }
        if ids.len() > 1 {
            return Err(StorageError::DataInconsistency {
                details: format!("Multiple genesis blocks found for epoch {}", epoch),
            });
        }

        Self::get(tx, ids.first().expect("length checked"))
    }

    /// Returns all blocks from and excluding the start block (lower height) to the end block (inclusive)
    pub fn get_all_blocks_between<TTx: StateStoreReadTransaction>(
        tx: &TTx,
        epoch: Epoch,
        start_block_height: NodeHeight,
        end_block_height: NodeHeight,
        include_dummy_blocks: bool,
        limit: usize,
    ) -> Result<Vec<Self>, StorageError> {
        tx.blocks_get_all_between(epoch, start_block_height, end_block_height, include_dummy_blocks, limit)
    }

    pub fn exists<TTx: StateStoreReadTransaction>(&self, tx: &TTx) -> Result<bool, StorageError> {
        Self::record_exists(tx, self.id())
    }

    pub fn parent_exists<TTx: StateStoreReadTransaction>(&self, tx: &TTx) -> Result<bool, StorageError> {
        Self::record_exists(tx, self.parent())
    }

    pub fn record_exists<TTx: StateStoreReadTransaction>(tx: &TTx, block_id: &BlockId) -> Result<bool, StorageError> {
        tx.blocks_exists(block_id)
    }

    pub fn insert<TTx: StateStoreWriteTransaction>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        tx.blocks_insert(self)
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

    pub fn remove_orphaned_blocks<TTx>(&self, tx: &mut TTx) -> Result<(), StorageError>
    where
        TTx: StateStoreWriteTransaction + Deref,
        TTx::Target: StateStoreReadTransaction,
    {
        let other_blocks = Self::get_ids_by_epoch_and_height(&**tx, self.epoch(), self.height())?;
        for block_id in other_blocks {
            if block_id == *self.id() {
                continue;
            }
            info!(
                target: LOG_TARGET,
                "❗️🔗 Removing orphaned block {} from epoch {} height {}",
                block_id,
                self.epoch(),
                self.height()
            );
            delete_orphaned_block_and_children(tx, &block_id)?;
        }
        Ok(())
    }

    pub fn lock_executions<TTx: StateStoreWriteTransaction>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        tx.block_transaction_executions_lock_any_for_block(&self.as_leaf())?;
        Ok(())
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

    pub fn commit_block_without_state_changes<TTx>(&self, tx: &mut TTx, commit_qc_id: &PcId) -> Result<(), StorageError>
    where
        TTx: StateStoreWriteTransaction + Deref,
        TTx::Target: StateStoreReadTransaction,
    {
        self.commit_block(tx, commit_qc_id, &HashMap::new())
    }

    pub fn commit_block<TTx>(
        &self,
        tx: &mut TTx,
        commit_qc_id: &PcId,
        version_updates: &HashMap<Shard, Version>,
    ) -> Result<(), StorageError>
    where
        TTx: StateStoreWriteTransaction + Deref,
        TTx::Target: StateStoreReadTransaction,
    {
        // Set the QC that caused this block to be committed, marking it as committed
        tx.blocks_set_qcs(self.id(), Some(commit_qc_id), None)?;

        if self.is_dummy() {
            info!(
                target: LOG_TARGET,
                "🍼 COMMIT dummy block {}", self
            );
            return Ok(());
        }

        let Some(block_diff) = self.get_diff(&**tx).optional()? else {
            info!(
                target: LOG_TARGET,
                "🌳 COMMIT block {} with no substate change(s)", self
            );

            // No diff to commit
            return Ok(());
        };

        // Consume the block diff
        block_diff.remove(tx)?;

        info!(
            target: LOG_TARGET,
            "🌳 COMMIT block {} with {} substate change(s)", self, block_diff.len()
        );

        let changes = block_diff.into_changes();

        let batch = changes.into_iter()
            .filter(|change| {
                if self.shard_group().contains_or_global(&change.shard()) {
                    true
                } else {
                    // This should have been filtered out already, but just in case
                    warn!(
                    target: LOG_TARGET,
                    "❓️ Skipping substate change {} for shard {} in block {} because it is not in the shard group {}",
                    change.as_change_string(),
                    change.shard(),
                    self.id(),
                    self.shard_group()
                );
                    false
                }
            })
            // Group by shard
            .try_fold(SubstateUpdateBatch::new(self.network(), self.epoch()), |mut batch, change| {
                let Some(state_version) =
                    version_updates
                        .get(&change.shard())
                        .copied() else {
                    // A panic may be more appropriate here, this should never happen
                    return Err(StorageError::DataInconsistency {
                        details: format!(
                            "NEVER HAPPEN: Shard state version for shard {} not found in block {}",
                            change.shard(),
                            self.id()
                        ),
                    });
                };

                batch.with_transition(change.shard(), state_version)
                    .push(change.into_transition());

                Ok(batch)
            })?;

        SubstateRecord::commit_batch(tx, batch)?;

        Ok(())
    }

    pub fn get_diff<TTx: StateStoreReadTransaction>(&self, tx: &TTx) -> Result<BlockDiff, StorageError> {
        tx.block_diffs_get(self.id())
    }

    pub fn add_justify_qc<TTx: StateStoreWriteTransaction>(
        &mut self,
        tx: &mut TTx,
        qc_id: &PcId,
    ) -> Result<(), StorageError> {
        self.justify_qc_id = Some(*qc_id);
        tx.blocks_set_qcs(self.id(), None, Some(qc_id))
    }

    /// Checks if this block extends the given ancestor block.
    ///
    /// ## Behaviour
    /// 1. if self.id == ancestor.id, then this returns false
    /// 2. if self.parent == ancestor.id, then this returns true
    /// 3. this only checks for uncommitted (pending) blocks and will return false if the block is committed (unless the
    ///    two blocks happen to be direct descendants i.e. point 2).
    /// 4. if self.parent does not exist, then false is returned.
    /// 5. if ancestor does not exist, an error is returned.
    pub fn extends_pending<TTx: StateStoreReadTransaction>(
        &self,
        tx: &TTx,
        ancestor: &BlockId,
    ) -> Result<bool, StorageError> {
        if self.id() == ancestor {
            return Ok(false);
        }
        if self.parent() == ancestor {
            return Ok(true);
        }
        // First check the parent here, if it does not exist, then this block cannot extend anything.
        if !Block::record_exists(tx, self.parent())? {
            return Ok(false);
        }

        tx.blocks_is_pending_ancestor(self.parent(), ancestor)
    }

    pub fn get_parent<TTx: StateStoreReadTransaction>(&self, tx: &TTx) -> Result<Block, StorageError> {
        if self.id().is_zero() && self.parent().is_zero() {
            return Err(StorageError::NotFound {
                item: "Block parent",
                key: self.parent().to_string(),
            });
        }

        Block::get(tx, self.parent())
    }

    pub fn get_transactions<TTx: StateStoreReadTransaction>(
        &self,
        tx: &TTx,
    ) -> Result<Vec<TransactionRecord>, StorageError> {
        let tx_ids = self.commands().iter().filter_map(|t| t.transaction().map(|t| t.id()));
        let (found, missing) = TransactionRecord::get_any(tx, tx_ids)?;
        if !missing.is_empty() {
            return Err(StorageError::NotFound {
                item: "Transaction",
                key: missing
                    .into_iter()
                    .map(|id| id.to_string())
                    .collect::<Vec<_>>()
                    .join(", "),
            });
        }

        Ok(found)
    }

    /// Returns the transactions that are/will be committed by this block when this block.
    pub fn get_committing_transactions<TTx: StateStoreReadTransaction>(
        &self,
        tx: &TTx,
    ) -> Result<Vec<TransactionRecord>, StorageError> {
        let tx_ids = self.commands().iter().filter_map(|t| t.committing()).map(|t| t.id());
        let (found, missing) = TransactionRecord::get_any(tx, tx_ids)?;
        if !missing.is_empty() {
            return Err(StorageError::NotFound {
                item: "Transaction (get_committed_transactions)",
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
        num_preshards: NumPreshards,
    ) -> Result<Vec<SubstateUpdateProof>, StorageError> {
        // The block diff is removed as soon as it is committed, so we need to reconstruct the substate updates from the
        // committed transactions. TODO: this does not include "implicit" state transitions i.e. validator fee
        // pools.
        let committed = self
            .commands()
            .iter()
            .filter_map(|c| c.committing())
            .filter(|t| t.decision.is_commit())
            .collect::<Vec<_>>();

        let mut updates = Vec::with_capacity(committed.len());
        for transaction in committed {
            let tx_rec = transaction.get_transaction(tx)?;
            let Some(execution) = tx_rec.get_finalized_execution(tx).optional()? else {
                continue;
            };
            // TODO: this is not completely correct but this is only used for block sync which is only used (for now) by
            // the indexer, Returning substates like this is good enough.
            let outputs = execution.resulting_outputs();
            let outputs = outputs
                .iter()
                .map(|lock| lock.versioned_substate_id().as_versioned_ref())
                .filter(|id| self.shard_group().contains_or_global(&id.to_shard(num_preshards)));

            let substates = SubstateRecord::get_all(tx, outputs)?;
            for substate in substates {
                if substate.is_destroyed() {
                    // This substate is destroyed. One of the following are possible:
                    // 1. The substate was destroyed by this transaction and created in an earlier transaction
                    // 2. The substate was created by this transaction and destroyed in a later transaction
                    // It isn't possible for a substate to be created and destroyed by the same transaction
                    // because the engine can never emit such a substate diff.
                    // TODO: This is currently not used - if we need this in future, we can include the state hash en
                    //       lieu of the actual state which does not exist
                    // if substate.created_by_transaction == transaction.id
                    // {     updates.push(SubstateUpdate::Create(SubstateCreate {
                    //         // created_qc: substate.get_created_quorum_certificate(tx)?,
                    //         substate: substate.try_into()?,
                    //     }));
                    // } else {
                    updates.push(SubstateUpdateProof::Destroy(SubstateDestroy {
                        substate_id: substate.substate_id.clone(),
                        version: substate.version,
                        // justify: ProposalCertificate::get(tx, &destroyed.justify)?,
                    }));
                } else {
                    updates.push(SubstateUpdateProof::Create(Box::new(SubstateCreate {
                        // created_qc: substate.get_created_quorum_certificate(tx)?,
                        substate: substate.into(),
                    })));
                };
            }
        }

        Ok(updates)
    }

    pub fn get_transaction_receipts<TTx: StateStoreReadTransaction>(
        &self,
        tx: &TTx,
    ) -> Result<Vec<SubstateCreate>, StorageError> {
        let committed = self
            .commands()
            .iter()
            .filter_map(|c| c.committing())
            .filter(|t| t.decision.is_commit());

        let receipt_ids = committed
            .map(|atom| TransactionReceiptAddress::from_array(atom.id.into_array()))
            .map(VersionedSubstateId::for_tx_receipt)
            .collect::<Vec<_>>();

        let receipts = SubstateRecord::get_all(tx, receipt_ids.iter().map(Into::into))?;
        let receipts = receipts
            .into_iter()
            .map(|receipt| {
                Ok::<_, StorageError>(SubstateCreate {
                    substate: receipt.into(),
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(receipts)
    }

    /// Returns the QC that justifies this block
    pub fn get_justify_qc<TTx: StateStoreReadTransaction>(
        &self,
        tx: &TTx,
    ) -> Result<ProposalCertificate, StorageError> {
        let justify_qc_id = self.justify_qc_id.as_ref().ok_or_else(|| StorageError::QueryError {
            reason: format!("get_justify_qc: Block {} has not been justified", self.id()),
        })?;
        tx.proposal_certificates_get(self.epoch(), justify_qc_id)
    }

    pub fn get_commit_qc<TTx: StateStoreReadTransaction>(&self, tx: &TTx) -> Result<ProposalCertificate, StorageError> {
        let commit_qc_id = self.commit_qc_id.as_ref().ok_or_else(|| StorageError::QueryError {
            reason: format!("get_commit_qc: Block {} has not been committed", self.as_leaf()),
        })?;
        tx.proposal_certificates_get(self.epoch(), commit_qc_id)
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

        // Liveness rules
        //  (qc.viewNumber > lockedQC.viewNumber)
        if self.max_certificate_height() > locked.height() {
            return Ok(true);
        }

        // Safety rule
        // (node extends from lockedQC.node)
        if self.extends_pending(tx, locked.block_id())? {
            return Ok(true);
        }

        info!(
            target: LOG_TARGET,
            "❌ Block {} does satisfy the liveness or safety rules of the safeNode predicate. Locked block {}",
            self,
            locked,
        );
        Ok(false)
    }

    pub fn is_epoch_end_proposed_in_chain<TTx: StateStoreReadTransaction>(
        &self,
        tx: &TTx,
    ) -> Result<bool, StorageError> {
        tx.is_block_in_end_of_epoch_chain(self.id())
    }

    pub fn get_block_pledge<TTx: StateStoreReadTransaction>(
        &self,
        tx: &TTx,
        for_shard_group: ShardGroup,
    ) -> Result<BlockPledge, StorageError> {
        if self.is_committed() {
            // TODO: we could preserve DOWN substates "for some reasonable time" (currently we do for the whole epoch so
            // this isnt a problem). However, this case applies to nodes that are catching up only
            // (otherwise the transaction would not be committed and therefore the pledges still available).
            // This would not be a concern if we were able to force commits without having to execute everything
            // historically.
            debug!(
                target: LOG_TARGET,
                "get_block_pledge: Block {} is already committed. Some substates may be DOWN and therefore these pledges will not be provided", self.as_leaf()
            );
        }

        let log_bool = |context: &str, atom: &TransactionAtom, val: bool| {
            if !val {
                debug!(
                    target: LOG_TARGET,
                    "get_block_pledge: Excluding {atom} because {context}"
                );
            }
            val
        };

        let applicable_transactions = self
            .commands()
            .iter()
            .filter_map(|c| {
                c.local_prepare()
                    // No need to broadcast LocalPrepare if the committee is output only (TODO: this no longer applies as output only skips LocalPrepare, so do we need this?)
                    .filter(|atom| log_bool("LocalPrepare, local output-only", atom, !atom.evidence.is_committee_output_only(self.shard_group())))
                    .or_else(|| {
                        // Avoid pledging twice - for input-involved SGs we have already sent pledges in LocalPrepare phase. For output-only, we need to pledge in the LocalAccept phase
                        c.local_accept()
                            .filter(|atom| log_bool("LocalAccept, foreign input-involved", atom, atom.evidence.is_committee_output_only(for_shard_group)))
                    })
            })
            .filter(|atom| log_bool("Is ABORT", atom, atom.decision.is_commit()))
            .filter(|atom| log_bool("Foreign SG not involved", atom, atom.evidence.has(&for_shard_group)));

        let mut num_applicable = 0;
        let mut pledges = BlockPledge::new();
        for atom in applicable_transactions {
            num_applicable += 1;
            let evidence = atom
                .evidence
                .get(&self.shard_group())
                .ok_or_else(|| StorageError::DataInconsistency {
                    details: format!("Local evidence for atom {} is missing in block {}", atom.id, self),
                })?;

            // TODO(perf): O(n) queries
            let substates = SubstateRecord::get_all(
                tx,
                evidence
                    .all_pledged_inputs_iter()
                    .map(|(substate_id, ev)| VersionedSubstateIdRef::new(substate_id, ev.version)),
            )?;

            debug!(
                target: LOG_TARGET,
                "get_block_pledge: {} locked for atom {} in block {}",
                substates.len(), atom.id, self
            );

            let self_as_leaf = self.as_leaf();
            for substate in substates {
                let version = substate.version();
                let id = substate.substate_id;
                let value = substate.substate_value.ok_or_else(|| StorageError::DataInconsistency {
                    details: format!(
                        "Pledge {}:{} has no substate value however a value is required",
                        id, version
                    ),
                })?;

                debug!(
                    target: LOG_TARGET,
                    "get_block_pledge: Adding pledge {}:{} for atom {} in block {}",
                    id, version, atom.id, self_as_leaf
                );
                pledges.add_substate_pledge(id, version, value);
            }
        }

        debug!(
            target: LOG_TARGET,
            "get_block_pledge: {num_applicable} pledge(s) for shard group {for_shard_group}"
        );

        Ok(pledges)
    }

    pub fn get_foreign_proposals<TTx: StateStoreReadTransaction>(
        &self,
        tx: &TTx,
    ) -> Result<Vec<ForeignProposalRecord>, StorageError> {
        ForeignProposalRecord::get_any(tx, self.all_foreign_proposals().map(|p| &p.block_id))
    }

    pub fn increment_leader_failure_count<TTx: StateStoreWriteTransaction>(
        &self,
        tx: &mut TTx,
        max_missed_proposal_cap: u64,
    ) -> Result<(), StorageError> {
        tx.validator_epoch_stats_updates(
            self.epoch(),
            iter::once(
                ValidatorStatsUpdate::new(self.proposed_by())
                    .add_missed_proposal()
                    .set_max_missed_proposals_cap(max_missed_proposal_cap),
            ),
        )
    }

    pub fn clear_leader_failure_count<TTx: StateStoreWriteTransaction>(
        &self,
        tx: &mut TTx,
    ) -> Result<(), StorageError> {
        tx.validator_epoch_stats_updates(
            self.epoch(),
            iter::once(ValidatorStatsUpdate::new(self.proposed_by()).reset_missed_proposals()),
        )
    }
}

impl Display for Block {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        if self.is_dummy() {
            write!(f, "Dummy")?;
        }
        write!(
            f,
            "[{}, justify: {} ({}), TC: {}, {}, {}, {} cmd(s){}, {}->{}]",
            self.height(),
            self.justify().height(),
            if self.timeout_certificate().is_none() && !self.is_dummy() {
                "🟢"
            } else {
                "🟡"
            },
            self.timeout_certificate.as_ref().map(|tc| tc.height()).display(),
            self.epoch(),
            self.shard_group(),
            self.commands().len(),
            if self.is_epoch_end() { " EOE" } else { "" },
            self.id(),
            self.parent()
        )
    }
}

/// Deletes everything related to a block as well as any child blocks
fn delete_orphaned_block_and_children<TTx>(tx: &mut TTx, block_id: &BlockId) -> Result<(), StorageError>
where
    TTx: StateStoreWriteTransaction + Deref,
    TTx::Target: StateStoreReadTransaction,
{
    let children = tx.blocks_get_pending_ids_by_parent(block_id)?;
    for child in children {
        delete_orphaned_block_and_children(tx, &child)?;
    }
    tx.block_diffs_remove(block_id).optional()?;
    tx.pending_state_tree_diffs_remove_by_block(block_id).optional()?;
    tx.substate_locks_remove_any_by_block_id(block_id)?;
    tx.transaction_pool_state_updates_remove_any_by_block_id(block_id)?;
    tx.block_transaction_executions_remove_any_by_block_id(block_id)?;
    tx.foreign_proposals_clear_proposed_in(block_id).optional()?;
    tx.lock_conflicts_remove_by_block_id(block_id)?;

    Block::delete_record(tx, block_id)?;

    Ok(())
}
