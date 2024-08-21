//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::str::FromStr;

use diesel::Queryable;
use tari_dan_common_types::Epoch;
use tari_dan_storage::{consensus_models, consensus_models::Decision, StorageError};
use tari_transaction::UnsignedTransaction;
use time::PrimitiveDateTime;

use crate::serialization::deserialize_json;

#[derive(Debug, Clone, Queryable)]
pub struct Transaction {
    pub id: i32,
    pub transaction_id: String,
    pub fee_instructions: String,
    pub instructions: String,
    pub signatures: String,
    pub inputs: String,
    pub filled_inputs: String,
    pub resolved_inputs: Option<String>,
    pub resulting_outputs: Option<String>,
    pub result: Option<String>,
    pub execution_time_ms: Option<i64>,
    pub final_decision: Option<String>,
    pub finalized_at: Option<PrimitiveDateTime>,
    pub abort_details: Option<String>,
    pub min_epoch: Option<i64>,
    pub max_epoch: Option<i64>,
    pub created_at: PrimitiveDateTime,
}

impl TryFrom<Transaction> for tari_transaction::Transaction {
    type Error = StorageError;

    fn try_from(value: Transaction) -> Result<Self, Self::Error> {
        let fee_instructions = deserialize_json(&value.fee_instructions)?;
        let instructions = deserialize_json(&value.instructions)?;
        let signatures = deserialize_json(&value.signatures)?;

        let inputs = deserialize_json(&value.inputs)?;

        let filled_inputs = deserialize_json(&value.filled_inputs)?;
        let min_epoch = value.min_epoch.map(|epoch| Epoch(epoch as u64));
        let max_epoch = value.max_epoch.map(|epoch| Epoch(epoch as u64));

        Ok(Self::new(
            UnsignedTransaction {
                fee_instructions,
                instructions,
                inputs,
                min_epoch,
                max_epoch,
            },
            signatures,
        )
        .with_filled_inputs(filled_inputs))
    }
}

impl TryFrom<Transaction> for consensus_models::TransactionRecord {
    type Error = StorageError;

    fn try_from(value: Transaction) -> Result<Self, Self::Error> {
        let final_decision = value
            .final_decision
            .as_deref()
            .map(Decision::from_str)
            .transpose()
            .map_err(|_| StorageError::DecodingError {
                operation: "TryFrom<Transaction> for consensus_models::ExecutedTransaction",
                item: "decision",
                details: format!(
                    "Failed to parse decision from string: {}",
                    value.final_decision.as_ref().unwrap()
                ),
            })?;
        let result = value.result.as_deref().map(deserialize_json).transpose()?;
        let resulting_outputs = value.resulting_outputs.as_deref().map(deserialize_json).transpose()?;
        let resolved_inputs = value.resolved_inputs.as_deref().map(deserialize_json).transpose()?;
        let abort_details = value.abort_details.as_deref().map(deserialize_json).transpose()?;

        let finalized_time = value
            .finalized_at
            .map(|t| t.assume_offset(time::UtcOffset::UTC) - value.created_at.assume_offset(time::UtcOffset::UTC))
            .map(|d| d.try_into().unwrap_or_default());

        Ok(Self::load(
            value.try_into()?,
            result,
            resolved_inputs,
            final_decision,
            finalized_time,
            resulting_outputs,
            abort_details,
        ))
    }
}

impl TryFrom<Transaction> for consensus_models::ExecutedTransaction {
    type Error = StorageError;

    fn try_from(value: Transaction) -> Result<Self, Self::Error> {
        let rec = consensus_models::TransactionRecord::try_from(value)?;

        if rec.execution_result.is_none() {
            return Err(StorageError::QueryError {
                reason: format!("Transaction {} has not executed", rec.transaction.id()),
            });
        }
        rec.try_into()
    }
}
