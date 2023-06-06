//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use diesel::Queryable;
use tari_dan_storage::{consensus_models, consensus_models::TransactionId, StorageError};
use time::PrimitiveDateTime;

use crate::{deser::deserialize_hex, error::SqliteStorageError};

#[derive(Debug, Clone, Queryable)]
pub struct TransactionDecision {
    pub id: i32,
    pub transaction_id: String,
    pub decision: String,
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
            decision: value
                .decision
                .parse()
                .map_err(|_| SqliteStorageError::MalformedDbData {
                    operation: "TryFrom<TransactionDecision> decision",
                    details: format!("{} is an invalid decision", value.decision),
                })?,
            per_shard_validator_fee: value.fee as u64,
        })
    }
}
