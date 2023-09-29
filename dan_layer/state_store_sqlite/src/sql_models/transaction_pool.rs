//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use diesel::Queryable;
use tari_dan_storage::{consensus_models, consensus_models::TransactionAtom, StorageError};
use time::PrimitiveDateTime;

use crate::serialization::{deserialize_hex_try_from, deserialize_json, parse_from_string};

#[derive(Debug, Clone, Queryable)]
pub struct TransactionPoolRecord {
    pub id: i32,
    pub transaction_id: String,
    pub involved_shards: String,
    pub original_decision: String,
    pub local_decision: Option<String>,
    pub remote_decision: Option<String>,
    pub evidence: String,
    pub transaction_fee: i64,
    pub leader_fee: i64,
    pub stage: String,
    pub pending_stage: Option<String>,
    pub is_ready: bool,
    pub updated_at: PrimitiveDateTime,
    pub created_at: PrimitiveDateTime,
}

impl TryFrom<TransactionPoolRecord> for consensus_models::TransactionPoolRecord {
    type Error = StorageError;

    fn try_from(value: TransactionPoolRecord) -> Result<Self, Self::Error> {
        Ok(Self::load(
            TransactionAtom {
                id: deserialize_hex_try_from(&value.transaction_id)?,
                decision: parse_from_string(&value.original_decision)?,
                evidence: deserialize_json(&value.evidence)?,
                transaction_fee: value.transaction_fee as u64,
                leader_fee: value.leader_fee as u64,
            },
            parse_from_string(&value.stage)?,
            value.pending_stage.as_deref().map(parse_from_string).transpose()?,
            value.local_decision.as_deref().map(parse_from_string).transpose()?,
            value.remote_decision.as_deref().map(parse_from_string).transpose()?,
            value.is_ready,
        ))
    }
}

#[derive(Debug, Clone, Queryable)]
pub struct TransactionPoolState {
    pub id: i32,
    pub block_id: String,
    pub block_height: i64,
    pub transaction_id: String,
    pub stage: String,
    pub is_ready: bool,
    pub created_at: PrimitiveDateTime,
}
