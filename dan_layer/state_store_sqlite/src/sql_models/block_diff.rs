//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use diesel::Queryable;
use tari_dan_common_types::shard::Shard;
use tari_dan_storage::{consensus_models, consensus_models::BlockId, StorageError};
use tari_transaction::VersionedSubstateId;
use time::PrimitiveDateTime;

use crate::serialization::{deserialize_hex_try_from, deserialize_json};

#[derive(Debug, Clone, Queryable)]
pub struct BlockDiff {
    pub id: i32,
    pub block_id: String,
    pub transaction_id: String,
    pub substate_id: String,
    pub version: i32,
    pub shard: i32,
    pub change: String,
    pub state: Option<String>,
    pub created_at: PrimitiveDateTime,
}

impl BlockDiff {
    pub fn try_load(block_id: BlockId, diff: Vec<Self>) -> Result<consensus_models::BlockDiff, StorageError> {
        let changes = diff
            .into_iter()
            .map(Self::try_convert_change)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(consensus_models::BlockDiff { block_id, changes })
    }

    pub fn try_convert_change(d: Self) -> Result<consensus_models::SubstateChange, StorageError> {
        let substate_id = d.substate_id.parse().map_err(|err| StorageError::DataInconsistency {
            details: format!("Invalid substate id {}: {}", d.substate_id, err),
        })?;
        let id = VersionedSubstateId::new(substate_id, d.version as u32);
        let transaction_id = deserialize_hex_try_from(&d.transaction_id)?;
        let shard = Shard::from(d.shard as u32);
        match d.change.as_str() {
            "Up" => {
                let state = d.state.ok_or(StorageError::DataInconsistency {
                    details: "Block diff change type is Up but state is missing".to_string(),
                })?;
                Ok(consensus_models::SubstateChange::Up {
                    id,
                    shard,
                    transaction_id,
                    substate: deserialize_json(&state)?,
                })
            },
            "Down" => Ok(consensus_models::SubstateChange::Down {
                id,
                transaction_id,
                shard,
            }),
            _ => Err(StorageError::DataInconsistency {
                details: format!("Invalid block diff change type: {}", d.change),
            }),
        }
    }
}
