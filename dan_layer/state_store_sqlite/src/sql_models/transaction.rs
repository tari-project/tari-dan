//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use diesel::Queryable;
use tari_common_types::types::PublicKey;
use tari_dan_storage::{consensus_models, consensus_models::TransactionId, StorageError};
use tari_utilities::ByteArray;
use time::PrimitiveDateTime;

use crate::{
    error::SqliteStorageError,
    serialization::{deserialize_hex, deserialize_json},
};

#[derive(Debug, Clone, Queryable)]
pub struct Transaction {
    pub id: i32,
    pub transaction_id: String,
    pub fee_instructions: String,
    pub instructions: String,
    pub sender_public_key: String,
    pub signature: String,
    pub meta: String,
    pub result: String,
    pub involved_shards: String,
    pub is_finalized: bool,
    pub created_at: PrimitiveDateTime,
}

impl TryFrom<Transaction> for consensus_models::Transaction {
    type Error = StorageError;

    fn try_from(value: Transaction) -> Result<Self, Self::Error> {
        Ok(Self {
            hash: TransactionId::try_from(deserialize_hex(&value.transaction_id)?).map_err(|e| {
                SqliteStorageError::MalformedDbData {
                    operation: "TryFrom<Transaction> transaction_id",
                    details: e.to_string(),
                }
            })?,
            fee_instructions: deserialize_json(&value.fee_instructions)?,
            instructions: deserialize_json(&value.instructions)?,
            signature: deserialize_json(&value.signature)?,
            sender_public_key: PublicKey::from_bytes(&deserialize_hex(&value.sender_public_key)?).map_err(|e| {
                SqliteStorageError::MalformedDbData {
                    operation: "TryFrom<Transaction> sender_public_key",
                    details: e.to_string(),
                }
            })?,
            meta: deserialize_json(&value.meta)?,
        })
    }
}
