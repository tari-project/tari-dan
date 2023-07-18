//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

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
    pub exists: String,
    pub outputs: String,
    pub filled_inputs: String,
    pub filled_outputs: String,
    pub result: String,
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
        let exists = deserialize_json(&value.exists)?;
        let outputs = deserialize_json(&value.outputs)?;
        let filled_inputs = deserialize_json(&value.filled_inputs)?;
        let filled_outputs = deserialize_json(&value.filled_outputs)?;

        Ok(Self::new(
            fee_instructions,
            instructions,
            signature,
            inputs,
            exists,
            outputs,
            filled_inputs,
            filled_outputs,
        ))
    }
}

impl TryFrom<Transaction> for consensus_models::ExecutedTransaction {
    type Error = StorageError;

    fn try_from(value: Transaction) -> Result<Self, Self::Error> {
        let is_finalized = value.is_finalized;
        let result = deserialize_json(&value.result)?;
        Ok(Self::new_with_finalized(value.try_into()?, result, is_finalized))
    }
}
