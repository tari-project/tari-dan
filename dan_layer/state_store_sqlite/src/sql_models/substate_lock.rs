//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use diesel::Queryable;
use tari_dan_storage::{consensus_models, StorageError};
use time::PrimitiveDateTime;

use crate::serialization::deserialize_hex_try_from;

#[derive(Debug, Clone, Queryable)]
pub struct SubstateLock {
    pub id: i32,
    pub block_id: String,
    pub transaction_id: String,
    pub substate_id: String,
    pub version: i32,
    pub lock: String,
    pub is_local_only: bool,
    pub created_at: PrimitiveDateTime,
}

impl SubstateLock {
    pub fn try_into_substate_lock(self) -> Result<consensus_models::LockedSubstate, StorageError> {
        let transaction_id = deserialize_hex_try_from(&self.transaction_id)?;
        let version = self.version as u32;
        let lock = self.lock.parse().map_err(|_| StorageError::DataInconsistency {
            details: format!("Failed to parse SubstateLockFlag: {}", self.lock),
        })?;
        let is_local_only = self.is_local_only;

        Ok(consensus_models::LockedSubstate::new(
            transaction_id,
            version,
            lock,
            is_local_only,
        ))
    }
}
