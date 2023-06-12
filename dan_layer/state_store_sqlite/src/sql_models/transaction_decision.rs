//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use diesel::Queryable;
use tari_dan_storage::{consensus_models, consensus_models::TransactionId, StorageError};
use time::PrimitiveDateTime;

use crate::{error::SqliteStorageError, serialization::deserialize_hex};

#[derive(Debug, Clone, Queryable)]
pub struct TransactionDecision {
    pub id: i32,
    pub transaction_id: String,
    pub overall_decision: String,
    pub transaction_decision: String,
    pub fee: i64,
    pub created_at: PrimitiveDateTime,
}

impl TryFrom<TransactionDecision> for consensus_models::TransactionDecision {
    type Error = StorageError;

    fn try_from(value: TransactionDecision) -> Result<Self, Self::Error> {
        Ok(consensus_models::TransactionDecision {
            transaction_id: TransactionId::try_from(deserialize_hex(&value.transaction_id)?).map_err(|e| {
                SqliteStorageError::MalformedDbData {
                    operation: "TryFrom<TransactionDecision> transaction_id",
                    details: e.to_string(),
                }
            })?,
            overall_decision: value
                .overall_decision
                .parse()
                .map_err(|_| SqliteStorageError::MalformedDbData {
                    operation: "TryFrom<TransactionDecision> decision",
                    details: format!("{} is an invalid decision", value.overall_decision),
                })?,
            transaction_decision: value.transaction_decision.parse().map_err(|_| {
                SqliteStorageError::MalformedDbData {
                    operation: "TryFrom<TransactionDecision> decision",
                    details: format!("{} is an invalid decision", value.transaction_decision),
                }
            })?,
            per_shard_validator_fee: value.fee as u64,
        })
    }
}
