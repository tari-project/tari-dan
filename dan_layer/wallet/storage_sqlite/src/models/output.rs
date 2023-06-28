//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use chrono::NaiveDateTime;
use tari_common_types::types::{Commitment, PublicKey};
use tari_dan_wallet_sdk::{models::ConfidentialOutputModel, storage::WalletStorageError};
use tari_template_lib::models::EncryptedData;
use tari_utilities::hex::Hex;

use crate::schema::outputs;

#[derive(Debug, Clone, Identifiable, Queryable)]
#[diesel(table_name = outputs)]
pub struct ConfidentialOutput {
    pub id: i32,
    pub account_id: i32,
    pub vault_id: i32,
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
    pub encrypted_data: Vec<u8>,
}

impl ConfidentialOutput {
    pub(crate) fn try_into_output(
        self,
        account_address_str: &str,
        vault_addr_str: &str,
    ) -> Result<ConfidentialOutputModel, WalletStorageError> {
        Ok(ConfidentialOutputModel {
            account_address: account_address_str
                .parse()
                .map_err(|_| WalletStorageError::DecodingError {
                    operation: "try_into_output",
                    item: "output",
                    details: format!("Corrupt db: invalid account address '{}'", account_address_str),
                })?,
            vault_address: vault_addr_str.parse().map_err(|_| WalletStorageError::DecodingError {
                operation: "try_into_output",
                item: "output",
                details: format!("Corrupt db: invalid vault address '{}'", vault_addr_str),
            })?,
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
            encrypted_data: EncryptedData::try_from(self.encrypted_data.as_slice()).map_err(|_| {
                WalletStorageError::DecodingError {
                    operation: "try_into_output",
                    item: "output",
                    details: "Corrupt db: invalid encrypted data".to_string(),
                }
            })?,
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
