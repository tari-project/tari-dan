//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use diesel::Queryable;
use tari_dan_storage::{consensus_models, StorageError};
use time::PrimitiveDateTime;

use crate::serialization::{deserialize_json, parse_from_string};

#[derive(Debug, Clone, Queryable)]
pub struct BurntUtxo {
    pub id: i32,
    pub substate_id: String,
    pub substate_value: String,
    pub base_layer_block_height: i64,
    pub proposed_in_block: Option<String>,
    pub proposed_in_block_height: Option<i64>,
    pub created_at: PrimitiveDateTime,
}

impl TryFrom<BurntUtxo> for consensus_models::BurntUtxo {
    type Error = StorageError;

    fn try_from(value: BurntUtxo) -> Result<Self, Self::Error> {
        Ok(Self {
            substate_id: parse_from_string(&value.substate_id)?,
            substate_value: deserialize_json(&value.substate_value)?,
            proposed_in_block: value.proposed_in_block.as_deref().map(deserialize_json).transpose()?,
            base_layer_block_height: value.base_layer_block_height as u64,
        })
    }
}
