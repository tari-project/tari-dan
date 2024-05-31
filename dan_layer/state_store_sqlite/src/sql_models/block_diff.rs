//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use diesel::Queryable;
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
    pub change: String,
    pub state: Option<String>,
    pub created_at: PrimitiveDateTime,
}

impl BlockDiff {
    pub fn try_load(block_id: BlockId, diff: Vec<Self>) -> Result<consensus_models::BlockDiff, StorageError> {
        let mut changes = Vec::with_capacity(diff.len());
        for d in diff {
            let substate_id = d.substate_id.parse().map_err(|err| StorageError::DataInconsistency {
                details: format!("Invalid substate id {}: {}", d.substate_id, err),
            })?;
            let id = VersionedSubstateId::new(substate_id, d.version as u32);
            let transaction_id = deserialize_hex_try_from(&d.transaction_id)?;
            match d.change.as_str() {
                "Up" => {
                    let state = d.state.ok_or(StorageError::DataInconsistency {
                        details: "Block diff change type is Up but state is missing".to_string(),
                    })?;
                    changes.push(consensus_models::SubstateChange::Up {
                        id,
                        transaction_id,
                        substate: deserialize_json(&state)?,
                    });
                },
                "Down" => {
                    changes.push(consensus_models::SubstateChange::Down { id, transaction_id });
                },
                _ => {
                    return Err(StorageError::DataInconsistency {
                        details: format!("Invalid block diff change type: {}", d.change),
                    });
                },
            }
        }
        Ok(consensus_models::BlockDiff { block_id, changes })
    }
}
