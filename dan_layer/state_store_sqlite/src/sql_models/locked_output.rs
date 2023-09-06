//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use diesel::Queryable;
use tari_dan_storage::{consensus_models, StorageError};
use time::PrimitiveDateTime;

use crate::serialization::deserialize_hex_try_from;

#[derive(Debug, Clone, Queryable)]
pub struct LockedOutput {
    pub id: i32,
    pub block_id: String,
    pub transaction_id: String,
    pub shard_id: String,
    pub created_at: PrimitiveDateTime,
}

impl TryFrom<LockedOutput> for consensus_models::LockedOutput {
    type Error = StorageError;

    fn try_from(value: LockedOutput) -> Result<Self, Self::Error> {
        Ok(Self {
            block_id: deserialize_hex_try_from(&value.block_id)?,
            transaction_id: deserialize_hex_try_from(&value.transaction_id)?,
            shard_id: deserialize_hex_try_from(&value.shard_id)?,
        })
    }
}
