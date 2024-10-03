//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use std::{
    fmt::{self, Display, Formatter},
    hash::Hash,
    ops::Deref,
    str::FromStr,
};

use serde::{Deserialize, Serialize};
use tari_dan_common_types::{Epoch, ShardGroup};

use super::{Block, BlockId, BlockPledge, QuorumCertificate};
use crate::{StateStoreReadTransaction, StateStoreWriteTransaction, StorageError};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ForeignProposal {
    pub block: Block,
    pub block_pledge: BlockPledge,
    pub justify_qc: QuorumCertificate,
    pub proposed_by_block: Option<BlockId>,
    pub status: ForeignProposalStatus,
}

impl ForeignProposal {
    pub fn new(block: Block, block_pledge: BlockPledge, justify_qc: QuorumCertificate) -> Self {
        Self {
            block,
            block_pledge,
            justify_qc,
            proposed_by_block: None,
            status: ForeignProposalStatus::New,
        }
    }

    pub fn to_atom(&self) -> ForeignProposalAtom {
        ForeignProposalAtom {
            shard_group: self.block.shard_group(),
            block_id: *self.block.id(),
        }
    }

    pub fn block(&self) -> &Block {
        &self.block
    }

    pub fn block_pledge(&self) -> &BlockPledge {
        &self.block_pledge
    }

    pub fn justify_qc(&self) -> &QuorumCertificate {
        &self.justify_qc
    }

    pub fn proposed_by_block(&self) -> Option<&BlockId> {
        self.proposed_by_block.as_ref()
    }

    pub fn status(&self) -> ForeignProposalStatus {
        self.status
    }
}

impl ForeignProposal {
    pub fn upsert<TTx>(&self, tx: &mut TTx, proposed_in_block: Option<BlockId>) -> Result<(), StorageError>
    where
        TTx: StateStoreWriteTransaction + Deref + ?Sized,
        TTx::Target: StateStoreReadTransaction,
    {
        self.justify_qc().save(tx)?;
        tx.foreign_proposals_upsert(self, proposed_in_block)
    }

    pub fn delete<TTx: StateStoreWriteTransaction + ?Sized>(
        tx: &mut TTx,
        block_id: &BlockId,
    ) -> Result<(), StorageError> {
        tx.foreign_proposals_delete(block_id)
    }

    pub fn delete_in_epoch<TTx: StateStoreWriteTransaction + ?Sized>(
        tx: &mut TTx,
        epoch: Epoch,
    ) -> Result<(), StorageError> {
        tx.foreign_proposals_delete_in_epoch(epoch)
    }

    pub fn get_any<'a, TTx: StateStoreReadTransaction + ?Sized, I: IntoIterator<Item = &'a BlockId>>(
        tx: &TTx,
        block_ids: I,
    ) -> Result<Vec<Self>, StorageError> {
        tx.foreign_proposals_get_any(block_ids)
    }

    pub fn exists<TTx: StateStoreReadTransaction + ?Sized>(&self, tx: &TTx) -> Result<bool, StorageError> {
        tx.foreign_proposals_exists(self.block.id())
    }

    pub fn get_all_new<TTx: StateStoreReadTransaction + ?Sized>(
        tx: &TTx,
        block_id: &BlockId,
        limit: usize,
    ) -> Result<Vec<Self>, StorageError> {
        tx.foreign_proposals_get_all_new(block_id, limit)
    }

    pub fn set_proposed_in<TTx: StateStoreWriteTransaction + ?Sized>(
        tx: &mut TTx,
        block_id: &BlockId,
        proposed_in_block: &BlockId,
    ) -> Result<(), StorageError> {
        tx.foreign_proposals_set_proposed_in(block_id, proposed_in_block)
    }

    pub fn has_unconfirmed<TTx: StateStoreReadTransaction + ?Sized>(
        tx: &TTx,
        epoch: Epoch,
    ) -> Result<bool, StorageError> {
        tx.foreign_proposals_has_unconfirmed(epoch)
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "../../bindings/src/types/")
)]
pub struct ForeignProposalAtom {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub block_id: BlockId,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub shard_group: ShardGroup,
}

impl ForeignProposalAtom {
    pub fn exists<TTx: StateStoreReadTransaction + ?Sized>(&self, tx: &TTx) -> Result<bool, StorageError> {
        tx.foreign_proposals_exists(&self.block_id)
    }

    pub fn get_proposal<TTx: StateStoreReadTransaction + ?Sized>(
        &self,
        tx: &TTx,
    ) -> Result<ForeignProposal, StorageError> {
        let mut found = tx.foreign_proposals_get_any(Some(&self.block_id))?;
        let found = found.pop().ok_or_else(|| StorageError::NotFound {
            item: "ForeignProposal".to_string(),
            key: self.block_id.to_string(),
        })?;
        Ok(found)
    }

    pub fn delete<TTx: StateStoreWriteTransaction + ?Sized>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        ForeignProposal::delete(tx, &self.block_id)
    }

    pub fn set_status<TTx: StateStoreWriteTransaction + ?Sized>(
        &self,
        tx: &mut TTx,
        status: ForeignProposalStatus,
    ) -> Result<(), StorageError> {
        tx.foreign_proposals_set_status(&self.block_id, status)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub enum ForeignProposalStatus {
    /// New foreign proposal that has not yet been proposed
    #[default]
    New,
    /// Foreign proposal has been proposed, but not yet locked.
    Proposed,
    /// Foreign proposal has been confirmed i.e. the block containing it has been locked.
    Confirmed,
}

impl Display for ForeignProposalStatus {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            ForeignProposalStatus::New => write!(f, "New"),
            ForeignProposalStatus::Proposed => write!(f, "Proposed"),
            ForeignProposalStatus::Confirmed => write!(f, "Confirmed"),
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
            _ => Err(StorageError::DecodingError {
                operation: "ForeignProposalStatus::from_str",
                item: "foreign proposal",
                details: format!("Invalid foreign proposal state {}", s),
            }),
        }
    }
}
