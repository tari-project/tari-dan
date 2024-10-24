//    Copyright 2024 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use diesel::Queryable;
use tari_dan_common_types::{shard::Shard, Epoch};
use tari_dan_storage::{
    consensus_models,
    consensus_models::{StateTransitionId, SubstateCreatedProof, SubstateData, SubstateDestroyedProof, SubstateUpdate},
    StorageError,
};
use time::PrimitiveDateTime;

use crate::sql_models::SubstateRecord;

#[derive(Debug, Clone, Queryable)]
pub struct StateTransition {
    pub id: i32,
    pub epoch: i64,
    pub shard: i32,
    pub seq: i64,
    #[allow(dead_code)]
    pub substate_address: String,
    #[allow(dead_code)]
    pub substate_id: String,
    #[allow(dead_code)]
    pub version: i32,
    pub transition: String,
    #[allow(dead_code)]
    pub state_hash: Option<String>,
    #[allow(dead_code)]
    pub state_version: i64,
    #[allow(dead_code)]
    pub created_at: PrimitiveDateTime,
}

impl StateTransition {
    pub fn try_convert(self, substate: SubstateRecord) -> Result<consensus_models::StateTransition, StorageError> {
        let substate = consensus_models::SubstateRecord::try_from(substate)?;
        let seq = self.seq as u64;
        let epoch = Epoch(self.epoch as u64);
        let shard = Shard::from(self.shard as u32);

        let update = match self.transition.as_str() {
            "UP" => SubstateUpdate::Create(SubstateCreatedProof {
                substate: SubstateData {
                    substate_id: substate.substate_id,
                    version: substate.version,
                    substate_value: substate.substate_value,
                    created_by_transaction: substate.created_by_transaction,
                },
            }),
            "DOWN" => {
                if !substate.is_destroyed() {
                    return Err(StorageError::DataInconsistency {
                        details: format!(
                            "State transition for substate {}:{} is DOWN but the substate is not destroyed",
                            substate.substate_id, substate.version
                        ),
                    });
                }

                SubstateUpdate::Destroy(SubstateDestroyedProof {
                    destroyed_by_transaction: substate.destroyed().unwrap().by_transaction,
                    substate_id: substate.substate_id,
                    version: substate.version,
                })
            },
            _ => {
                return Err(StorageError::DataInconsistency {
                    details: format!(
                        "StateTransition::try_convert: '{}' is not a valid transition",
                        self.transition
                    ),
                })
            },
        };

        Ok(consensus_models::StateTransition {
            id: StateTransitionId::new(epoch, shard, seq),
            update,
        })
    }
}
