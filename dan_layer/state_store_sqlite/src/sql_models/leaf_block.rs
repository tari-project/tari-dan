//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use diesel::Queryable;
use tari_dan_common_types::{Epoch, NodeHeight};
use tari_dan_storage::{consensus_models, consensus_models::BlockId, StorageError};
use time::PrimitiveDateTime;

use crate::{error::SqliteStorageError, serialization::deserialize_hex};

#[derive(Debug, Clone, Queryable)]
pub struct LeafBlock {
    pub id: i32,
    pub epoch: i64,
    pub block_id: String,
    pub block_height: i64,
    pub created_at: PrimitiveDateTime,
}

impl TryFrom<LeafBlock> for consensus_models::LeafBlock {
    type Error = StorageError;

    fn try_from(value: LeafBlock) -> Result<Self, Self::Error> {
        Ok(Self {
            epoch: Epoch(value.epoch as u64),
            block_id: BlockId::try_from(deserialize_hex(&value.block_id)?).map_err(|e| {
                SqliteStorageError::MalformedDbData {
                    operation: "TryFrom<LeafBlock> block_id",
                    details: e.to_string(),
                }
            })?,
            height: NodeHeight(value.block_height as u64),
        })
    }
}
