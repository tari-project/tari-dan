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
    pub inputs: String,
    pub exists: String,
    pub outputs: String,
    pub result: String,
    pub is_finalized: bool,
    pub created_at: PrimitiveDateTime,
}

impl TryFrom<Transaction> for consensus_models::Transaction {
    type Error = StorageError;

    fn try_from(value: Transaction) -> Result<Self, Self::Error> {
        let hash = TransactionId::try_from(deserialize_hex(&value.transaction_id)?).map_err(|e| {
            SqliteStorageError::MalformedDbData {
                operation: "TryFrom<Transaction> transaction_id",
                details: e.to_string(),
            }
        })?;
        let fee_instructions = deserialize_json(&value.fee_instructions)?;
        let instructions = deserialize_json(&value.instructions)?;
        let signature = deserialize_json(&value.signature)?;

        let sender_public_key = PublicKey::from_bytes(&deserialize_hex(&value.sender_public_key)?).map_err(|e| {
            SqliteStorageError::MalformedDbData {
                operation: "TryFrom<Transaction> sender_public_key",
                details: e.to_string(),
            }
        })?;

        let inputs = deserialize_json(&value.inputs)?;
        let exists = deserialize_json(&value.exists)?;
        let outputs = deserialize_json(&value.outputs)?;

        Ok(Self::new(
            hash,
            fee_instructions,
            instructions,
            signature,
            sender_public_key,
            inputs,
            exists,
            outputs,
        ))
    }
}

impl TryFrom<Transaction> for consensus_models::ExecutedTransaction {
    type Error = StorageError;

    fn try_from(value: Transaction) -> Result<Self, Self::Error> {
        let result = deserialize_json(&value.result)?;
        Ok(Self::new(value.try_into()?, result))
    }
}
