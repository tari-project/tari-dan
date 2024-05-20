//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use std::{
    fmt::{self, Display, Formatter},
    hash::Hash,
    str::FromStr,
};

use serde::{Deserialize, Serialize};
use tari_dan_common_types::{shard::Shard, NodeHeight};
use tari_transaction::TransactionId;
#[cfg(feature = "ts")]
use ts_rs::TS;

use super::BlockId;
use crate::{StateStoreReadTransaction, StateStoreWriteTransaction, StorageError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub enum ForeignProposalState {
    New,
    Proposed,
    Deleted,
}

impl Display for ForeignProposalState {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            ForeignProposalState::New => write!(f, "New"),
            ForeignProposalState::Proposed => write!(f, "Proposed"),
            ForeignProposalState::Deleted => write!(f, "Deleted"),
        }
    }
}

impl FromStr for ForeignProposalState {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "New" => Ok(ForeignProposalState::New),
            "Proposed" => Ok(ForeignProposalState::Proposed),
            "Deleted" => Ok(ForeignProposalState::Deleted),
            _ => Err(anyhow::anyhow!("Invalid foreign proposal state {}", s)),
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct ForeignProposal {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub bucket: Shard,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub block_id: BlockId,
    pub state: ForeignProposalState,
    pub proposed_height: Option<NodeHeight>,
    #[cfg_attr(feature = "ts", ts(type = "Array<string>"))]
    pub transactions: Vec<TransactionId>,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub base_layer_block_height: u64,
}

impl ForeignProposal {
    pub fn new(
        bucket: Shard,
        block_id: BlockId,
        transactions: Vec<TransactionId>,
        base_layer_block_height: u64,
    ) -> Self {
        Self {
            bucket,
            block_id,
            state: ForeignProposalState::New,
            proposed_height: None,
            transactions,
            base_layer_block_height,
        }
    }

    pub fn set_proposed_height(&mut self, height: NodeHeight) -> &mut Self {
        self.proposed_height = Some(height);
        self.state = ForeignProposalState::Proposed;
        self
    }
}

impl ForeignProposal {
    pub fn upsert<TTx: StateStoreWriteTransaction + ?Sized>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        tx.foreign_proposal_upsert(self)?;
        Ok(())
    }

    pub fn delete<TTx: StateStoreWriteTransaction + ?Sized>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        tx.foreign_proposal_delete(self)?;
        Ok(())
    }

    pub fn exists<TTx: StateStoreReadTransaction + ?Sized>(
        tx: &TTx,
        foreign_proposal: &Self,
    ) -> Result<bool, StorageError> {
        tx.foreign_proposal_exists(foreign_proposal)
    }

    pub fn get_all_new<TTx: StateStoreReadTransaction + ?Sized>(tx: &TTx) -> Result<Vec<Self>, StorageError> {
        tx.foreign_proposal_get_all_new()
    }

    pub fn get_all_pending<TTx: StateStoreReadTransaction + ?Sized>(
        tx: &TTx,
        from_block_id: &BlockId,
        to_block_id: &BlockId,
    ) -> Result<Vec<Self>, StorageError> {
        tx.foreign_proposal_get_all_pending(from_block_id, to_block_id)
    }

    pub fn get_all_proposed<TTx: StateStoreReadTransaction + ?Sized>(
        tx: &TTx,
        to_height: NodeHeight,
    ) -> Result<Vec<Self>, StorageError> {
        tx.foreign_proposal_get_all_proposed(to_height)
    }
}
