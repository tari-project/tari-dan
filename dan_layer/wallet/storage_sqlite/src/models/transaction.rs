//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::str::FromStr;

use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use tari_common_types::types::PublicKey;
use tari_dan_common_types::{Epoch, ShardId};
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
    pub json_result: Option<String>,
    pub qcs: Option<String>,
    pub final_fee: Option<i64>,
    pub status: String,
    pub is_dry_run: bool,
    pub min_epoch: Option<i64>,
    pub max_epoch: Option<i64>,
    pub updated_at: NaiveDateTime,
    pub created_at: NaiveDateTime,
}

/// Struct used to keep inputs and outputs in a single field as json
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionInputs {
    pub inputs: Vec<ShardId>,
    pub input_refs: Vec<ShardId>,
}

impl Transaction {
    pub fn try_into_wallet_transaction(self) -> Result<WalletTransaction<PublicKey>, WalletStorageError> {
        let signature = deserialize_json(&self.signature)?;
        let sender_public_key =
            Hex::from_hex(&self.sender_public_key).map_err(|e| WalletStorageError::DecodingError {
                operation: "transaction_get",
                item: "sender_address",
                details: e.to_string(),
            })?;
        let signature = TransactionSignature::new(sender_public_key, signature);
        let TransactionInputs { inputs, input_refs } = deserialize_json(&self.meta)?;

        Ok(WalletTransaction {
            transaction: tari_transaction::Transaction::new(
                deserialize_json(&self.fee_instructions)?,
                deserialize_json(&self.instructions)?,
                signature,
                inputs,
                input_refs,
                vec![],
                self.min_epoch.map(|epoch| Epoch(epoch as u64)),
                self.max_epoch.map(|epoch| Epoch(epoch as u64)),
            ),
            status: TransactionStatus::from_str(&self.status).map_err(|e| WalletStorageError::DecodingError {
                operation: "transaction_get",
                item: "status",
                details: e.to_string(),
            })?,
            finalize: self.result.as_deref().map(deserialize_json).transpose()?,
            final_fee: self.final_fee.map(|f| f.into()),
            qcs: self.qcs.map(|q| deserialize_json(&q)).transpose()?.unwrap_or_default(),
            is_dry_run: self.is_dry_run,
            // TODO: This is always None
            json_result: self
                .json_result
                .map(|r| deserialize_json(&r))
                .transpose()?
                .unwrap_or_default(),
        })
    }
}
