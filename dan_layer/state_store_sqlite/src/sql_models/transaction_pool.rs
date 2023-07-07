//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use diesel::Queryable;
use tari_dan_storage::{consensus_models, consensus_models::TransactionAtom, StorageError};
use time::PrimitiveDateTime;

use crate::serialization::{deserialize_hex_try_from, deserialize_json};

#[derive(Debug, Clone, Queryable)]
pub struct TransactionPoolRecord {
    pub id: i32,
    pub transaction_id: String,
    pub involved_shards: String,
    pub overall_decision: String,
    pub evidence: String,
    pub fee: i64,
    pub stage: String,
    pub is_ready: bool,
    pub created_at: PrimitiveDateTime,
}

impl TryFrom<TransactionPoolRecord> for consensus_models::TransactionPoolRecord {
    type Error = StorageError;

    fn try_from(value: TransactionPoolRecord) -> Result<Self, Self::Error> {
        Ok(Self {
            transaction: TransactionAtom {
                id: deserialize_hex_try_from(&value.transaction_id)?,
                involved_shards: deserialize_json(&value.involved_shards)?,
                decision: value
                    .overall_decision
                    .parse()
                    .map_err(|_| StorageError::DecodingError {
                        operation: "TryFrom TransactionPoolRecord",
                        item: "decision",
                        details: format!("{} is an invalid decision", value.overall_decision),
                    })?,
                evidence: deserialize_json(&value.evidence)?,
                fee: value.fee as u64,
            },
            stage: value.stage.parse().map_err(|_| StorageError::DecodingError {
                operation: "TryFrom TransactionPoolRecord",
                item: "stage",
                details: format!("{} is an invalid stage", value.stage),
            })?,
            is_ready: value.is_ready,
        })
    }
}
