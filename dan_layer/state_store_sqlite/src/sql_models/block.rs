//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use diesel::Queryable;
use tari_dan_common_types::{Epoch, NodeHeight};
use tari_dan_storage::{consensus_models, StorageError};
use time::PrimitiveDateTime;

use crate::deser::{deserialize_hex_try_from, deserialize_json};

#[derive(Debug, Clone, Queryable)]
pub struct Block {
    pub id: i32,
    pub block_id: String,
    pub parent_block_id: String,
    pub height: i64,
    pub leader_round: i64,
    pub epoch: i64,
    pub proposed_by: String,
    pub justify: String,
    pub prepared: String,
    pub precommitted: String,
    pub committed: String,
    pub created_at: PrimitiveDateTime,
}

impl TryFrom<Block> for consensus_models::Block {
    type Error = StorageError;

    fn try_from(value: Block) -> Result<Self, Self::Error> {
        Ok(Self::new(
            deserialize_hex_try_from(&value.parent_block_id)?,
            deserialize_json(&value.justify)?,
            NodeHeight(value.height as u64),
            Epoch(value.epoch as u64),
            value.leader_round as u64,
            deserialize_hex_try_from(&value.proposed_by)?,
            deserialize_json(&value.prepared)?,
            deserialize_json(&value.precommitted)?,
            deserialize_json(&value.committed)?,
        ))
    }
}
