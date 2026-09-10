//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use std::{
    fmt::{self, Display, Formatter},
    hash::Hash,
    ops::Deref,
    str::FromStr,
};

use borsh::BorshSerialize;
use serde::{Deserialize, Serialize};
use tari_common_types::types::FixedHash;
use tari_consensus_types::{BlockId, LeafBlock};
use tari_crypto::tari_utilities::ByteArray;
use tari_ootle_common_types::{Epoch, NodeHeight, ShardGroup, committee::CommitteeInfo};
use tari_ootle_transaction::TransactionId;
use tari_sidechain::QuorumCertificate;
use tari_template_lib_types::crypto::RistrettoPublicKeyBytes;

use super::{BlockPledge, Command, CommandOrHash, CommandsCommitProof, LockedEpoch};
use crate::{StateStoreReadTransaction, StateStoreWriteTransaction, StorageError};

#[derive(Debug, Clone, Deserialize, Serialize, minicbor::Encode, minicbor::Decode, minicbor::CborLen)]
pub struct ForeignProposalRecord {
    #[n(0)]
    block_id: BlockId,
    #[n(1)]
    proposal: ForeignProposal,
    #[n(2)]
    proposed_in_block: Option<BlockId>,
    #[n(3)]
    status: ForeignProposalStatus,
}

impl ForeignProposalRecord {
    pub fn new(proposal: ForeignProposal) -> Self {
        let block_id = proposal.calculate_block_id();
        Self {
            block_id,
            proposal,
            proposed_in_block: None,
            status: ForeignProposalStatus::New,
        }
    }

    pub fn load(
        block_id: BlockId,
        proposal: ForeignProposal,
        proposed_in_block: Option<BlockId>,
        status: ForeignProposalStatus,
    ) -> Self {
        Self {
            block_id,
            proposal,
            proposed_in_block,
            status,
        }
    }

    /// Returns the atom for this proposal.
    pub fn to_atom(&self) -> ForeignProposalAtom {
        ForeignProposalAtom {
            shard_group: self.proposal.shard_group_unchecked(),
            block_id: *self.block_id(),
        }
    }

    pub fn block_id(&self) -> &BlockId {
        &self.block_id
    }

    pub fn as_leaf(&self) -> LeafBlock {
        LeafBlock {
            block_id: self.block_id,
            height: self.height(),
            epoch: self.epoch(),
            shard_group: self.shard_group_unchecked(),
        }
    }

    pub fn proposal(&self) -> &ForeignProposal {
        &self.proposal
    }

    pub fn into_proposal(self) -> ForeignProposal {
        self.proposal
    }

    pub fn commands(&self) -> &[CommandOrHash] {
        self.proposal.commit_proof().commands()
    }

    pub fn full_commands_iter(&self) -> impl Iterator<Item = &Command> + '_ {
        self.commands().iter().filter_map(|cmd| cmd.command())
    }

    pub fn shard_group_checked(&self) -> Option<ShardGroup> {
        self.proposal.shard_group_checked()
    }

    /// Returns the shard group for the proposal proof.
    /// The shard group bounds are not checked.
    pub fn shard_group_unchecked(&self) -> ShardGroup {
        self.proposal.shard_group_unchecked()
    }

    pub fn height(&self) -> NodeHeight {
        self.proposal.height()
    }

    pub fn epoch(&self) -> Epoch {
        self.proposal.epoch()
    }

    pub fn block_pledge(&self) -> &BlockPledge {
        self.proposal.block_pledge()
    }

    pub fn status(&self) -> ForeignProposalStatus {
        self.status
    }

    pub fn proposed_in_block(&self) -> Option<&BlockId> {
        self.proposed_in_block.as_ref()
    }

    /// Resets the proposal status to `New` and clears the `proposed_in_block`.
    pub fn reset_proposed(&mut self) -> &mut Self {
        self.proposed_in_block = None;
        self.status = ForeignProposalStatus::New;
        self
    }

    pub fn set_proposal_status(&mut self, status: ForeignProposalStatus) -> &mut Self {
        self.status = status;
        self
    }

    pub fn set_proposed_in_block(&mut self, block_id: BlockId) -> &mut Self {
        self.proposed_in_block = Some(block_id);
        self
    }
}

impl ForeignProposalRecord {
    pub fn save<TTx>(&self, tx: &mut TTx) -> Result<(), StorageError>
    where
        TTx: StateStoreWriteTransaction + Deref,
        TTx::Target: StateStoreReadTransaction,
    {
        tx.foreign_proposals_save(self)
    }

    pub fn update_status<TTx: StateStoreWriteTransaction>(
        &mut self,
        tx: &mut TTx,
        status: ForeignProposalStatus,
        set_proposed_in_block: Option<&BlockId>,
    ) -> Result<(), StorageError> {
        self.status = status;
        if let Some(proposed_in_block) = set_proposed_in_block {
            self.proposed_in_block = Some(*proposed_in_block);
        }
        tx.foreign_proposals_set_status(self.block_id(), status, set_proposed_in_block)
    }

    pub fn set_status_by_id<TTx: StateStoreWriteTransaction>(
        tx: &mut TTx,
        block_id: &BlockId,
        status: ForeignProposalStatus,
        set_proposed_in_block: Option<&BlockId>,
    ) -> Result<(), StorageError> {
        tx.foreign_proposals_set_status(block_id, status, set_proposed_in_block)
    }

    pub fn delete<TTx: StateStoreWriteTransaction>(tx: &mut TTx, block_id: &BlockId) -> Result<(), StorageError> {
        tx.foreign_proposals_delete(block_id)
    }

    pub fn get_any<'a, TTx: StateStoreReadTransaction, I: IntoIterator<Item = &'a BlockId>>(
        tx: &TTx,
        block_ids: I,
    ) -> Result<Vec<Self>, StorageError> {
        tx.foreign_proposals_get_any(block_ids)
    }

    pub fn exists<TTx: StateStoreReadTransaction>(&self, tx: &TTx) -> Result<bool, StorageError> {
        Self::record_exists(tx, self.block_id())
    }

    pub fn record_exists<TTx: StateStoreReadTransaction>(tx: &TTx, block_id: &BlockId) -> Result<bool, StorageError> {
        tx.foreign_proposals_exists(block_id)
    }

    pub fn get_all_new<TTx: StateStoreReadTransaction>(
        tx: &TTx,
        block_id: &BlockId,
        limit: usize,
    ) -> Result<Vec<Self>, StorageError> {
        tx.foreign_proposals_get_all_new(block_id, limit)
    }

    pub fn has_unconfirmed<TTx: StateStoreReadTransaction>(tx: &TTx, epoch: Epoch) -> Result<bool, StorageError> {
        tx.foreign_proposals_has_unconfirmed(epoch)
    }
}

impl Display for ForeignProposalRecord {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "ForeignProposalRecord({}, {}, cmds: {}, plg: {}, {}, {})",
            self.block_id,
            self.shard_group_unchecked(),
            self.proposal.commit_proof().commands().len(),
            self.proposal.block_pledge().len(),
            self.status,
            self.proposed_in_block
                .as_ref()
                .map_or_else(|| "None".to_string(), |id| id.to_string())
        )
    }
}

#[derive(
    Debug,
    Clone,
    Eq,
    PartialEq,
    Hash,
    Serialize,
    Deserialize,
    PartialOrd,
    Ord,
    BorshSerialize,
    minicbor::Encode,
    minicbor::Decode,
    minicbor::CborLen,
)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct ForeignProposalAtom {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    #[n(0)]
    pub block_id: BlockId,
    #[n(1)]
    pub shard_group: ShardGroup,
}

impl ForeignProposalAtom {
    pub fn exists<TTx: StateStoreReadTransaction>(&self, tx: &TTx) -> Result<bool, StorageError> {
        tx.foreign_proposals_exists(&self.block_id)
    }

    pub fn get_proposal<TTx: StateStoreReadTransaction>(
        &self,
        tx: &TTx,
    ) -> Result<ForeignProposalRecord, StorageError> {
        let mut found = tx.foreign_proposals_get_any(Some(&self.block_id))?;
        let found = found.pop().ok_or_else(|| StorageError::NotFound {
            item: "ForeignProposal",
            key: self.block_id.to_string(),
        })?;
        Ok(found)
    }

    pub fn delete<TTx: StateStoreWriteTransaction>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        ForeignProposalRecord::delete(tx, &self.block_id)
    }

    pub fn set_status<TTx: StateStoreWriteTransaction>(
        &self,
        tx: &mut TTx,
        status: ForeignProposalStatus,
        set_proposed_in: Option<&BlockId>,
    ) -> Result<(), StorageError> {
        tx.foreign_proposals_set_status(&self.block_id, status, set_proposed_in)
    }
}

#[derive(
    Debug,
    Clone,
    Copy,
    Serialize,
    Deserialize,
    Default,
    Eq,
    PartialEq,
    minicbor::Encode,
    minicbor::Decode,
    minicbor::CborLen,
)]
pub enum ForeignProposalStatus {
    /// New foreign proposal that has not yet been proposed
    #[default]
    #[n(0)]
    New,
    /// Foreign proposal has been proposed, but not yet locked.
    #[n(1)]
    Proposed,
    /// Foreign proposal has been confirmed i.e. the block containing it has been locked.
    #[n(2)]
    Confirmed,
    /// Foreign proposal has been rejected.
    #[n(3)]
    Invalid,
}

impl ForeignProposalStatus {
    pub fn is_new(&self) -> bool {
        matches!(self, ForeignProposalStatus::New)
    }

    pub fn is_proposed(&self) -> bool {
        matches!(self, ForeignProposalStatus::Proposed)
    }

    pub fn is_confirmed(&self) -> bool {
        matches!(self, ForeignProposalStatus::Confirmed)
    }

    pub fn is_invalid(&self) -> bool {
        matches!(self, ForeignProposalStatus::Invalid)
    }

    pub fn is_unconfirmed(&self) -> bool {
        matches!(self, ForeignProposalStatus::New | ForeignProposalStatus::Proposed)
    }
}

impl Display for ForeignProposalStatus {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            ForeignProposalStatus::New => write!(f, "New"),
            ForeignProposalStatus::Proposed => write!(f, "Proposed"),
            ForeignProposalStatus::Confirmed => write!(f, "Confirmed"),
            ForeignProposalStatus::Invalid => write!(f, "Invalid"),
        }
    }
}

impl FromStr for ForeignProposalStatus {
    type Err = StorageError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "New" => Ok(ForeignProposalStatus::New),
            "Proposed" => Ok(ForeignProposalStatus::Proposed),
            "Confirmed" => Ok(ForeignProposalStatus::Confirmed),
            "Invalid" => Ok(ForeignProposalStatus::Invalid),
            _ => Err(StorageError::DecodingError {
                operation: "ForeignProposalStatus::from_str",
                item: "foreign proposal",
                details: format!("Invalid foreign proposal state {}", s),
            }),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, minicbor::Encode, minicbor::Decode, minicbor::CborLen)]
pub struct ForeignProposal {
    #[n(0)]
    commit_proof: CommandsCommitProof,
    #[n(1)]
    block_pledge: BlockPledge,
}

impl ForeignProposal {
    pub fn new(commit_proof: CommandsCommitProof, block_pledge: BlockPledge) -> Self {
        Self {
            commit_proof,
            block_pledge,
        }
    }

    pub fn to_atom(&self) -> ForeignProposalAtom {
        ForeignProposalAtom {
            shard_group: self.shard_group_unchecked(),
            block_id: self
                .commit_proof
                .sidechain_block_commit_proof()
                .header
                .calculate_block_id()
                .into(),
        }
    }

    pub fn commit_proof(&self) -> &CommandsCommitProof {
        &self.commit_proof
    }

    pub fn block_pledge(&self) -> &BlockPledge {
        &self.block_pledge
    }

    pub fn get_justify_qc(&self) -> Option<&QuorumCertificate> {
        self.commit_proof
            .sidechain_block_commit_proof()
            // Should be the last QC in the commit proof
            .last_qc()
    }

    pub fn epoch(&self) -> Epoch {
        Epoch(self.commit_proof.sidechain_block_commit_proof().header.epoch)
    }

    pub fn height(&self) -> NodeHeight {
        NodeHeight(self.commit_proof.sidechain_block_commit_proof().header.height)
    }

    pub fn epoch_hash(&self) -> &FixedHash {
        &self.commit_proof.sidechain_block_commit_proof().header().epoch_hash
    }

    pub fn to_locked_epoch(&self) -> LockedEpoch {
        LockedEpoch::new(self.epoch(), self.epoch_hash().into_array().into())
    }

    pub fn proposed_by(&self) -> RistrettoPublicKeyBytes {
        RistrettoPublicKeyBytes::from_bytes(
            self.commit_proof
                .sidechain_block_commit_proof()
                .header
                .proposed_by
                .as_bytes(),
        )
        .expect("CompressedPublicKey is not 32 bytes")
    }

    pub fn network_byte(&self) -> u8 {
        self.commit_proof.sidechain_block_commit_proof().header.network
    }

    pub fn shard_group_checked(&self) -> Option<ShardGroup> {
        ShardGroup::new_checked(
            self.commit_proof
                .sidechain_block_commit_proof()
                .header
                .shard_group
                .start,
            self.commit_proof
                .sidechain_block_commit_proof()
                .header
                .shard_group
                .end_inclusive,
        )
    }

    pub fn shard_group_unchecked(&self) -> ShardGroup {
        ShardGroup::new_unchecked(
            self.commit_proof
                .sidechain_block_commit_proof()
                .header
                .shard_group
                .start,
            self.commit_proof
                .sidechain_block_commit_proof()
                .header
                .shard_group
                .end_inclusive,
        )
    }

    pub fn calculate_block_id(&self) -> BlockId {
        self.commit_proof
            .sidechain_block_commit_proof()
            .header
            .calculate_block_id()
            .into()
    }

    pub fn all_transaction_ids_in_committee<'a>(
        &'a self,
        committee_info: &'a CommitteeInfo,
    ) -> impl Iterator<Item = &'a TransactionId> + Clone + 'a {
        self.commit_proof
            .commands()
            .iter()
            .filter_map(|cmd| cmd.command())
            .filter_map(|cmd| cmd.transaction())
            .filter(|t| t.evidence.has_and_not_empty(&committee_info.shard_group()))
            .map(|t| t.id())
    }

    pub fn commands(&self) -> &[CommandOrHash] {
        self.commit_proof().commands()
    }

    pub fn full_commands_iter(&self) -> impl Iterator<Item = &Command> + '_ {
        self.commands().iter().filter_map(|cmd| cmd.command())
    }

    pub fn into_parts(self) -> (CommandsCommitProof, BlockPledge) {
        (self.commit_proof, self.block_pledge)
    }
}

impl Display for ForeignProposal {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "ForeignProposal({}, {} command(s), {} pledge(s))",
            self.commit_proof.calculate_block_id(),
            self.commit_proof.commands().len(),
            self.block_pledge.len()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tripwire, not coverage of the deletion itself: `foreign_proposals_set_status` drops the
    /// unconfirmed index entry only when a status leaves the unconfirmed set, and
    /// `foreign_proposals_get_all_new` iterates that index. Adding `Invalid` to the unconfirmed set would
    /// silently make a rejected proposal selectable again.
    #[test]
    fn a_rejected_proposal_leaves_the_unconfirmed_set() {
        assert!(!ForeignProposalStatus::Invalid.is_unconfirmed());
        assert!(!ForeignProposalStatus::Confirmed.is_unconfirmed());
        assert!(ForeignProposalStatus::New.is_unconfirmed());
        assert!(ForeignProposalStatus::Proposed.is_unconfirmed());
    }
}
