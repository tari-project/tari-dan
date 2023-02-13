//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::str::FromStr;

use chrono::NaiveDateTime;
use tari_dan_wallet_sdk::{
    models::{TransactionStatus, WalletTransaction},
    storage::WalletStorageError,
};
use tari_utilities::hex::Hex;

use crate::{schema::transactions, serialization::deserialize_json};

#[derive(Debug, Clone, Queryable, Identifiable)]
#[table_name = "transactions"]
pub struct Transaction {
    pub id: i32,
    pub hash: String,
    pub instructions: String,
    pub signature: String,
    pub sender_address: String,
    pub fee: i64,
    pub meta: String,
    pub result: Option<String>,
    pub qcs: Option<String>,
    pub status: String,
    pub updated_at: NaiveDateTime,
    pub created_at: NaiveDateTime,
}

impl Transaction {
    pub fn try_into_wallet_transaction(self) -> Result<WalletTransaction, WalletStorageError> {
        let signature = deserialize_json(&self.signature)?;
        let sender_address = Hex::from_hex(&self.sender_address).map_err(|e| WalletStorageError::DecodingError {
            operation: "transaction_get",
            item: "sender_address",
            details: e.to_string(),
        })?;

        Ok(WalletTransaction {
            transaction: tari_transaction::Transaction::new(
                self.fee as u64,
                deserialize_json(&self.instructions)?,
                signature,
                sender_address,
                deserialize_json(&self.meta)?,
            ),
            status: TransactionStatus::from_str(&self.status).map_err(|e| WalletStorageError::DecodingError {
                operation: "transaction_get",
                item: "status",
                details: e.to_string(),
            })?,
            result: self.result.map(|r| deserialize_json(&r)).transpose()?,
            qcs: self.qcs.map(|q| deserialize_json(&q)).transpose()?.unwrap_or_default(),
        })
    }
}
