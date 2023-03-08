//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use chrono::NaiveDateTime;
use tari_common_types::types::{Commitment, PublicKey};
use tari_dan_wallet_sdk::{models::ConfidentialOutput, storage::WalletStorageError};
use tari_utilities::hex::Hex;

use crate::schema::outputs;

#[derive(Debug, Clone, Identifiable, Queryable)]
#[diesel(table_name = outputs)]
pub struct ConfidentialOutputModel {
    pub id: i32,
    pub account_id: i32,
    pub commitment: String,
    pub value: i64,
    pub sender_public_nonce: Option<String>,
    pub secret_key_index: i64,
    pub public_asset_tag: Option<String>,
    pub status: String,
    pub locked_at: Option<NaiveDateTime>,
    pub locked_by_proof: Option<i32>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

impl ConfidentialOutputModel {
    pub fn try_into_output(self, account_name: String) -> Result<ConfidentialOutput, WalletStorageError> {
        Ok(ConfidentialOutput {
            account_name,
            commitment: Commitment::from_hex(&self.commitment).map_err(|_| WalletStorageError::DecodingError {
                operation: "outputs_lock_smallest_amount",
                item: "output commitment",
                details: "Corrupt db: invalid hex representation".to_string(),
            })?,
            value: self.value as u64,
            sender_public_nonce: self
                .sender_public_nonce
                .map(|nonce| PublicKey::from_hex(&nonce).unwrap()),
            secret_key_index: self.secret_key_index as u64,
            public_asset_tag: self.public_asset_tag.map(|tag| PublicKey::from_hex(&tag).unwrap()),
            status: self.status.parse().map_err(|_| WalletStorageError::DecodingError {
                operation: "try_into_output",
                item: "output",
                details: format!("Corrupt db: invalid output status '{}'", self.status),
            })?,
            locked_by_proof: self.locked_by_proof.map(|proof| proof as u64),
        })
    }
}
