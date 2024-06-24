//    Copyright 2024 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use time::PrimitiveDateTime;
use tari_dan_common_types::Epoch;
use tari_dan_common_types::shard::Shard;
use tari_dan_storage::{consensus_models, StorageError};
use crate::serialization::{deserialize_hex_try_from, deserialize_json};

pub struct EpochCheckpoint {
    id: i32,
    epoch: i64,
    shard: i32,
    block_id: String,
    state_root: String,
    qcs: String,
    created_at: PrimitiveDateTime,
}

impl TryFrom<EpochCheckpoint> for consensus_models::EpochCheckpoint {
    type Error = StorageError;

    fn try_from(e: EpochCheckpoint) -> Result<consensus_models::EpochCheckpoint, Self::Error> {
        let block_id = deserialize_hex_try_from(&e.block_id)?;
        let state_root = deserialize_hex_try_from(&e.state_root)?;
        let qcs = deserialize_json(&e.qcs)?;
        let shard= Shard::from(e.shard.try_into().map_err(|_| StorageError::DataInconsistency { details: format!("Invalid shard: {}", e.shard), })?);
        let epoch = Epoch(e.epoch.try_into().map_err(|_| StorageError::DataInconsistency { details: format!("Invalid epoch: {}", e.epoch), })?;
        Ok(consensus_models::EpochCheckpoint::new(
            block_id,
            epoch,
            shard,
            state_root,
            qcs,
        ))
    }
}