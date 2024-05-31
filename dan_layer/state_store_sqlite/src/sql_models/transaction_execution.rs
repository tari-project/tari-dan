//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::time::Duration;

use diesel::Queryable;
use tari_dan_storage::{consensus_models, StorageError};
use time::PrimitiveDateTime;

use crate::serialization::{deserialize_hex_try_from, deserialize_json};

#[derive(Debug, Clone, Queryable)]
pub struct TransactionExecution {
    pub id: i32,
    pub block_id: String,
    pub transaction_id: String,
    pub resolved_inputs: String,
    pub resulting_outputs: String,
    pub result: String,
    pub execution_time_ms: i64,
    pub created_at: PrimitiveDateTime,
}

impl TryFrom<TransactionExecution> for consensus_models::TransactionExecution {
    type Error = StorageError;

    fn try_from(value: TransactionExecution) -> Result<Self, Self::Error> {
        let block_id = deserialize_hex_try_from(&value.block_id)?;
        let transaction_id = deserialize_hex_try_from(&value.transaction_id)?;
        let execution_time = Duration::from_millis(value.execution_time_ms as u64);
        let result = deserialize_json(&value.result)?;
        let resulting_outputs = deserialize_json(&value.resulting_outputs)?;
        let resolved_inputs = deserialize_json(&value.resolved_inputs)?;

        Ok(Self::new(
            block_id,
            transaction_id,
            result,
            resolved_inputs,
            resulting_outputs,
            execution_time,
        ))
    }
}
