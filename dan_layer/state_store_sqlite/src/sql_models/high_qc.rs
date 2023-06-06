//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use diesel::Queryable;
use tari_dan_common_types::{Epoch, NodeHeight};
use tari_dan_storage::{consensus_models, StorageError};
use time::PrimitiveDateTime;

use crate::deser::deserialize_hex_try_from;

#[derive(Debug, Clone, Queryable)]
pub struct HighQc {
    pub id: i32,
    pub epoch: i64,
    pub block_id: String,
    pub block_height: i64,
    pub created_at: PrimitiveDateTime,
}

impl TryFrom<HighQc> for consensus_models::HighQc {
    type Error = StorageError;

    fn try_from(value: HighQc) -> Result<Self, Self::Error> {
        Ok(Self {
            epoch: Epoch(value.epoch as u64),
            block_id: deserialize_hex_try_from(&value.block_id)?,
            height: NodeHeight(value.block_height as u64),
        })
    }
}
