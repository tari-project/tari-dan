//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use diesel::Queryable;
use tari_dan_storage::{consensus_models, StorageError};
use time::PrimitiveDateTime;

use crate::serialization::{deserialize_hex_try_from, deserialize_json};

#[derive(Debug, Clone, Queryable)]
pub struct PendingStateTreeDiff {
    pub id: i32,
    pub block_id: String,
    pub block_height: i64,
    pub diff: String,
    pub created_at: PrimitiveDateTime,
}

impl TryFrom<PendingStateTreeDiff> for consensus_models::PendingStateTreeDiff {
    type Error = StorageError;

    fn try_from(value: PendingStateTreeDiff) -> Result<Self, Self::Error> {
        let block_id = deserialize_hex_try_from(&value.block_id)?;
        let block_height = value.block_height as u64;
        let diff = deserialize_json(&value.diff)?;
        Ok(Self::new(block_id, block_height.into(), diff))
    }
}
