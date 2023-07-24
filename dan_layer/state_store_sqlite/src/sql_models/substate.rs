//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use diesel::Queryable;
use tari_dan_common_types::{Epoch, NodeHeight};
use tari_dan_storage::{consensus_models, StorageError};
use time::PrimitiveDateTime;

use crate::serialization::{deserialize_hex_try_from, deserialize_json, parse_from_string};

#[derive(Debug, Clone, Queryable)]
pub struct SubstateRecord {
    pub id: i32,
    pub shard_id: String,
    pub address: String,
    pub version: i32,
    pub data: String,
    pub state_hash: String,
    pub created_by_transaction: String,
    pub created_justify: String,
    pub created_block: String,
    pub created_height: i64,
    pub destroyed_by_transaction: Option<String>,
    pub destroyed_justify: Option<String>,
    pub destroyed_by_block: Option<String>,
    pub created_at_epoch: i64,
    pub destroyed_at_epoch: Option<i64>,
    pub read_locks: i32,
    pub is_locked_w: bool,
    pub locked_by: Option<String>,
    pub created_at: PrimitiveDateTime,
    pub destroyed_at: Option<PrimitiveDateTime>,
}

impl TryFrom<SubstateRecord> for consensus_models::SubstateRecord {
    type Error = StorageError;

    fn try_from(value: SubstateRecord) -> Result<Self, Self::Error> {
        Ok(Self {
            address: parse_from_string(&value.address)?,
            version: value.version as u32,
            substate_value: deserialize_json(&value.data)?,
            state_hash: deserialize_hex_try_from(&value.state_hash)?,
            created_by_transaction: deserialize_hex_try_from(&value.created_by_transaction)?,
            created_justify: deserialize_hex_try_from(&value.created_justify)?,
            created_block: deserialize_hex_try_from(&value.created_block)?,
            created_height: NodeHeight(value.created_height as u64),
            destroyed_by_transaction: value
                .destroyed_by_transaction
                .map(|x| deserialize_hex_try_from(&x))
                .transpose()?,
            destroyed_justify: value
                .destroyed_justify
                .as_deref()
                .map(deserialize_hex_try_from)
                .transpose()?,
            destroyed_by_block: value
                .destroyed_by_block
                .as_deref()
                .map(deserialize_hex_try_from)
                .transpose()?,
            created_at_epoch: Epoch(value.created_at_epoch as u64),
            destroyed_at_epoch: value.destroyed_at_epoch.map(|x| Epoch(x as u64)),
        })
    }
}
