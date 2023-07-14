//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::str::FromStr;

use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use tari_dan_common_types::ShardId;
use tari_dan_wallet_sdk::{
    models::{TransactionStatus, WalletTransaction},
    storage::WalletStorageError,
};
use tari_transaction::TransactionSignature;
use tari_utilities::hex::Hex;

use crate::{schema::transactions, serialization::deserialize_json};

#[derive(Debug, Clone, Queryable, Identifiable)]
#[diesel(table_name = transactions)]
pub struct Transaction {
    pub id: i32,
    pub hash: String,
    pub instructions: String,
    pub signature: String,
    pub sender_public_key: String,
    pub fee_instructions: String,
    pub meta: String,
    pub result: Option<String>,
    pub transaction_failure: Option<String>,
    pub qcs: Option<String>,
    pub final_fee: Option<i64>,
    pub status: String,
    pub is_dry_run: bool,
    pub updated_at: NaiveDateTime,
    pub created_at: NaiveDateTime,
}

/// Struct used to keep inputs and outputs in a single field as json
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputsAndOutputs {
    pub inputs: Vec<ShardId>,
    pub input_refs: Vec<ShardId>,
    pub outputs: Vec<ShardId>,
}

impl Transaction {
    pub fn try_into_wallet_transaction(self) -> Result<WalletTransaction, WalletStorageError> {
        let signature = deserialize_json(&self.signature)?;
        let sender_public_key =
            Hex::from_hex(&self.sender_public_key).map_err(|e| WalletStorageError::DecodingError {
                operation: "transaction_get",
                item: "sender_address",
                details: e.to_string(),
            })?;
        let signature = TransactionSignature::new(sender_public_key, signature);
        let InputsAndOutputs {
            inputs,
            input_refs,
            outputs,
        } = deserialize_json(&self.meta)?;

        Ok(WalletTransaction {
            transaction: tari_transaction::Transaction::new(
                deserialize_json(&self.fee_instructions)?,
                deserialize_json(&self.instructions)?,
                signature,
                inputs,
                input_refs,
                outputs,
                vec![],
                vec![],
            ),
            status: TransactionStatus::from_str(&self.status).map_err(|e| WalletStorageError::DecodingError {
                operation: "transaction_get",
                item: "status",
                details: e.to_string(),
            })?,
            finalize: self.result.map(|r| deserialize_json(&r)).transpose()?,
            transaction_failure: self.transaction_failure.map(|r| deserialize_json(&r)).transpose()?,
            final_fee: self.final_fee.map(|f| f.into()),
            qcs: self.qcs.map(|q| deserialize_json(&q)).transpose()?.unwrap_or_default(),
            is_dry_run: self.is_dry_run,
        })
    }
}
