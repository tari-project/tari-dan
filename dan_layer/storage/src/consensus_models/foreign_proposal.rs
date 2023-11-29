//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use std::{
    fmt::{self, Display, Formatter},
    str::FromStr,
};

use serde::{Deserialize, Serialize};
use tari_dan_common_types::{shard_bucket::ShardBucket, NodeHeight};

use super::BlockId;
use crate::{StateStoreReadTransaction, StateStoreWriteTransaction, StorageError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
pub enum ForeignProposalState {
    New,
    Mined,
    Deleted,
}

impl Display for ForeignProposalState {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            ForeignProposalState::New => write!(f, "New"),
            ForeignProposalState::Mined => write!(f, "Mined"),
            ForeignProposalState::Deleted => write!(f, "Deleted"),
        }
    }
}

impl FromStr for ForeignProposalState {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "New" => Ok(ForeignProposalState::New),
            "Mined" => Ok(ForeignProposalState::Mined),
            "Deleted" => Ok(ForeignProposalState::Deleted),
            _ => Err(anyhow::anyhow!("Invalid foreign proposal state {}", s)),
        }
    }
}

#[derive(Debug, Clone, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
pub struct ForeignProposal {
    pub bucket: ShardBucket,
    pub block_id: BlockId,
    pub state: ForeignProposalState,
    pub mined_at: Option<NodeHeight>,
}

impl PartialEq for ForeignProposal {
    fn eq(&self, other: &Self) -> bool {
        self.bucket == other.bucket && self.block_id == other.block_id
    }
}

impl ForeignProposal {
    pub fn new(bucket: ShardBucket, block_id: BlockId) -> Self {
        Self {
            bucket,
            block_id,
            state: ForeignProposalState::New,
            mined_at: None,
        }
    }

    pub fn set_mined_at(mut self, mined_at: NodeHeight) -> Self {
        self.mined_at = Some(mined_at);
        self.state = ForeignProposalState::Mined;
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
        tx: &mut TTx,
        foreign_proposal: &Self,
    ) -> Result<bool, StorageError> {
        tx.foreign_proposal_exists(foreign_proposal)
    }

    pub fn get_all_new<TTx: StateStoreReadTransaction + ?Sized>(tx: &mut TTx) -> Result<Vec<Self>, StorageError> {
        tx.foreign_proposal_get_all_new()
    }

    pub fn get_all_pending<TTx: StateStoreReadTransaction + ?Sized>(
        tx: &mut TTx,
        from_block_id: &BlockId,
        to_block_id: &BlockId,
    ) -> Result<Vec<Self>, StorageError> {
        tx.foreign_proposal_get_all_pending(from_block_id, to_block_id)
    }
}
