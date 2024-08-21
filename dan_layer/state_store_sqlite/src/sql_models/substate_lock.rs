//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::str::FromStr;

use diesel::Queryable;
use tari_dan_storage::{consensus_models, StorageError};
use tari_engine_types::substate::SubstateId;
use time::PrimitiveDateTime;

use crate::{serialization::deserialize_hex_try_from, sql_models::SubstateRecord};

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
    pub fn try_into_substate_lock(self) -> Result<consensus_models::SubstateLock, StorageError> {
        let transaction_id = deserialize_hex_try_from(&self.transaction_id)?;
        let version = self.version as u32;
        let lock = self.lock.parse().map_err(|_| StorageError::DataInconsistency {
            details: format!("Failed to parse SubstateLockFlag: {}", self.lock),
        })?;
        let is_local_only = self.is_local_only;

        Ok(consensus_models::SubstateLock::new(
            transaction_id,
            version,
            lock,
            is_local_only,
        ))
    }

    pub fn try_into_locked_substate_value(
        self,
        substate_rec: Option<SubstateRecord>,
    ) -> Result<consensus_models::LockedSubstateValue, StorageError> {
        let substate_rec = substate_rec
            .map(consensus_models::SubstateRecord::try_from)
            .transpose()?;

        let id = SubstateId::from_str(&self.substate_id).map_err(|e| StorageError::DataInconsistency {
            details: format!(
                "[try_into_locked_substate_value] '{}' is not a valid SubstateId: {}",
                self.substate_id, e
            ),
        })?;
        Ok(consensus_models::LockedSubstateValue {
            locked_by_block: deserialize_hex_try_from(&self.block_id)?,
            substate_id: id,
            lock: self.try_into_substate_lock()?,
            value: substate_rec.map(|r| r.into_substate_value()),
        })
    }
}
