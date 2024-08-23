//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use diesel::Queryable;
use tari_dan_storage::{consensus_models, StorageError};
use time::PrimitiveDateTime;

use crate::serialization::deserialize_json;

#[derive(Debug, Clone, Queryable)]
pub struct ForeignParkedBlock {
    pub id: i32,
    pub block_id: String,
    pub block: String,
    pub block_pledges: String,
    pub justify_qc: String,
    pub created_at: PrimitiveDateTime,
}

impl TryFrom<ForeignParkedBlock> for consensus_models::ForeignParkedProposal {
    type Error = StorageError;

    fn try_from(value: ForeignParkedBlock) -> Result<Self, Self::Error> {
        let block = deserialize_json(&value.block)?;
        let block_pledge = deserialize_json(&value.block_pledges)?;
        let justify_qc = deserialize_json(&value.justify_qc)?;

        Ok(consensus_models::ForeignParkedProposal::new(
            consensus_models::ForeignProposal::new(block, block_pledge, justify_qc),
        ))
    }
}
