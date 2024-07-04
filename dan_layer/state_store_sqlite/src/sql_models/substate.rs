//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use diesel::Queryable;
use tari_dan_common_types::{shard::Shard, Epoch, NodeHeight};
use tari_dan_storage::{consensus_models, consensus_models::SubstateDestroyed, StorageError};
use time::PrimitiveDateTime;

use crate::serialization::{deserialize_hex_try_from, deserialize_json, parse_from_string};

#[derive(Debug, Clone, Queryable)]
pub struct SubstateRecord {
    pub id: i32,
    pub address: String,
    pub substate_id: String,
    pub version: i32,
    pub data: String,
    pub state_hash: String,
    pub created_by_transaction: String,
    pub created_justify: String,
    pub created_block: String,
    pub created_height: i64,
    pub created_at_epoch: i64,
    pub created_by_shard: i32,
    pub destroyed_by_transaction: Option<String>,
    pub destroyed_justify: Option<String>,
    pub destroyed_by_block: Option<i64>,
    pub destroyed_at_epoch: Option<i64>,
    pub destroyed_by_shard: Option<i32>,
    pub created_at: PrimitiveDateTime,
    pub destroyed_at: Option<PrimitiveDateTime>,
}

impl TryFrom<SubstateRecord> for consensus_models::SubstateRecord {
    type Error = StorageError;

    fn try_from(value: SubstateRecord) -> Result<Self, Self::Error> {
        let destroyed = value
            .destroyed_by_transaction
            .map(|tx_id| {
                Ok::<_, StorageError>(SubstateDestroyed {
                    by_transaction: deserialize_hex_try_from(&tx_id)?,
                    justify: deserialize_hex_try_from(value.destroyed_justify.as_deref().ok_or_else(|| {
                        StorageError::DataInconsistency {
                            details: "destroyed_justify not provided".to_string(),
                        }
                    })?)?,
                    by_block: value.destroyed_by_block.map(|v| NodeHeight(v as u64)).ok_or_else(|| {
                        StorageError::DataInconsistency {
                            details: "destroyed_by_block not provided".to_string(),
                        }
                    })?,
                    at_epoch: value.destroyed_at_epoch.map(|x| Epoch(x as u64)).ok_or_else(|| {
                        StorageError::DataInconsistency {
                            details: "destroyed_at_epoch not provided".to_string(),
                        }
                    })?,
                    by_shard: value.destroyed_by_shard.map(|x| Shard::from(x as u32)).ok_or_else(|| {
                        StorageError::DataInconsistency {
                            details: "destroyed_by_shard not provided".to_string(),
                        }
                    })?,
                })
            })
            .transpose()?;

        Ok(Self {
            substate_id: parse_from_string(&value.substate_id)?,
            version: value.version as u32,
            substate_value: deserialize_json(&value.data)?,
            state_hash: deserialize_hex_try_from(&value.state_hash)?,
            created_by_transaction: deserialize_hex_try_from(&value.created_by_transaction)?,
            created_justify: deserialize_hex_try_from(&value.created_justify)?,
            created_block: deserialize_hex_try_from(&value.created_block)?,
            created_height: NodeHeight(value.created_height as u64),
            destroyed,
            created_at_epoch: Epoch(value.created_at_epoch as u64),
            created_by_shard: Shard::from(value.created_by_shard as u32),
        })
    }
}
