//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::time::Duration;

use diesel::Queryable;
use tari_dan_storage::{consensus_models, StorageError};
use time::PrimitiveDateTime;

use crate::serialization::deserialize_json;

#[derive(Debug, Clone, Queryable)]
pub struct Transaction {
    pub id: i32,
    pub transaction_id: String,
    pub fee_instructions: String,
    pub instructions: String,
    pub signature: String,
    pub inputs: String,
    pub input_refs: String,
    pub outputs: String,
    pub filled_inputs: String,
    pub filled_outputs: String,
    pub result: Option<String>,
    pub execution_time_ms: Option<i64>,
    pub is_finalized: bool,
    pub created_at: PrimitiveDateTime,
}

impl TryFrom<Transaction> for tari_transaction::Transaction {
    type Error = StorageError;

    fn try_from(value: Transaction) -> Result<Self, Self::Error> {
        let fee_instructions = deserialize_json(&value.fee_instructions)?;
        let instructions = deserialize_json(&value.instructions)?;
        let signature = deserialize_json(&value.signature)?;

        let inputs = deserialize_json(&value.inputs)?;
        let input_refs = deserialize_json(&value.input_refs)?;
        let outputs = deserialize_json(&value.outputs)?;
        let filled_inputs = deserialize_json(&value.filled_inputs)?;
        let filled_outputs = deserialize_json(&value.filled_outputs)?;

        Ok(Self::new(
            fee_instructions,
            instructions,
            signature,
            inputs,
            input_refs,
            outputs,
            filled_inputs,
            filled_outputs,
        ))
    }
}

impl TryFrom<Transaction> for consensus_models::TransactionRecord {
    type Error = StorageError;

    fn try_from(value: Transaction) -> Result<Self, Self::Error> {
        let is_finalized = value.is_finalized;
        let execution_time = value.execution_time_ms.map(|ms| Duration::from_millis(ms as u64));
        let result = value.result.as_deref().map(deserialize_json).transpose()?;
        Ok(Self::new_with_details(
            value.try_into()?,
            result,
            execution_time,
            is_finalized,
        ))
    }
}

impl TryFrom<Transaction> for consensus_models::ExecutedTransaction {
    type Error = StorageError;

    fn try_from(value: Transaction) -> Result<Self, Self::Error> {
        let rec = consensus_models::TransactionRecord::try_from(value)?;

        if rec.result.is_none() {
            return Err(StorageError::QueryError {
                reason: format!("Transaction {} has not executed", rec.transaction.id()),
            });
        }
        rec.try_into()
    }
}
