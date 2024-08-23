//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use std::{
    fmt::{self, Display, Formatter},
    hash::Hash,
    ops::Deref,
    str::FromStr,
};

use serde::{Deserialize, Serialize};
use tari_dan_common_types::ShardGroup;

use super::{Block, BlockId, BlockPledge, QuorumCertificate};
use crate::{StateStoreReadTransaction, StateStoreWriteTransaction, StorageError};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ForeignProposal {
    pub block: Block,
    pub block_pledge: BlockPledge,
    pub justify_qc: QuorumCertificate,
    pub proposed_by_block: Option<BlockId>,
}

impl ForeignProposal {
    pub fn new(block: Block, block_pledge: BlockPledge, justify_qc: QuorumCertificate) -> Self {
        Self {
            block,
            block_pledge,
            justify_qc,
            proposed_by_block: None,
        }
    }

    pub fn to_atom(&self) -> ForeignProposalAtom {
        ForeignProposalAtom {
            shard_group: self.block.shard_group(),
            block_id: *self.block.id(),
            base_layer_block_height: self.block.base_layer_block_height(),
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

    pub fn delete<TTx: StateStoreWriteTransaction + ?Sized>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        tx.foreign_proposals_delete(self.block.id())
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
        max_base_layer_block_height: u64,
        block_id: &BlockId,
        limit: usize,
    ) -> Result<Vec<Self>, StorageError> {
        tx.foreign_proposals_get_all_new(max_base_layer_block_height, block_id, limit)
    }

    pub fn set_proposed_in<TTx: StateStoreWriteTransaction + ?Sized>(
        tx: &mut TTx,
        block_id: &BlockId,
        proposed_in_block: &BlockId,
    ) -> Result<(), StorageError> {
        tx.foreign_proposals_set_proposed_in(block_id, proposed_in_block)
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
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub base_layer_block_height: u64,
}

impl ForeignProposalAtom {
    pub fn exists<TTx: StateStoreReadTransaction + ?Sized>(&self, tx: &TTx) -> Result<bool, StorageError> {
        tx.foreign_proposals_exists(&self.block_id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
pub enum ForeignProposalStatus {
    New,
    Proposed,
    Deleted,
}

impl Display for ForeignProposalStatus {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            ForeignProposalStatus::New => write!(f, "New"),
            ForeignProposalStatus::Proposed => write!(f, "Proposed"),
            ForeignProposalStatus::Deleted => write!(f, "Deleted"),
        }
    }
}

impl FromStr for ForeignProposalStatus {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "New" => Ok(ForeignProposalStatus::New),
            "Proposed" => Ok(ForeignProposalStatus::Proposed),
            "Deleted" => Ok(ForeignProposalStatus::Deleted),
            _ => Err(anyhow::anyhow!("Invalid foreign proposal state {}", s)),
        }
    }
}
