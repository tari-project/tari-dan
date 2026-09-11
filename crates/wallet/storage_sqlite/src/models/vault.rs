//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::str::FromStr;

use diesel::{Identifiable, Queryable};
use tari_ootle_wallet_sdk::storage::WalletStorageError;
use tari_template_lib_types::{Amount, ComponentAddress, ResourceAddress, ResourceType, VaultId};
use time::PrimitiveDateTime;

use crate::schema::vaults;

#[derive(Debug, Clone, Queryable, Identifiable)]
#[diesel(table_name = vaults)]
pub struct Vault {
    pub id: i32,
    pub account_id: i32,
    pub address: String,
    pub resource_address: String,
    pub resource_type: String,
    pub revealed_balance: String,
    pub confidential_balance: String,
    pub token_symbol: Option<String>,
    pub divisibility: i32,
    pub created_at: PrimitiveDateTime,
    pub updated_at: PrimitiveDateTime,
    pub vault_version: i64,
}

impl Vault {
    pub(crate) fn try_into_vault(
        self,
        account_address: ComponentAddress,
        locked_revealed_balance: Amount,
    ) -> Result<tari_ootle_wallet_sdk::models::VaultModel, WalletStorageError> {
        Ok(tari_ootle_wallet_sdk::models::VaultModel {
            account_address,
            id: VaultId::from_str(&self.address).map_err(|e| WalletStorageError::DecodingError {
                operation: "try_into_vault",
                item: "vault.address",
                details: e.to_string(),
            })?,
            vault_version: u64::try_from(self.vault_version).map_err(|e| WalletStorageError::DecodingError {
                operation: "try_into_vault",
                item: "vault.vault_version",
                details: e.to_string(),
            })?,
            resource_address: ResourceAddress::from_str(&self.resource_address).map_err(|e| {
                WalletStorageError::DecodingError {
                    operation: "try_into_vault",
                    item: "vault.resource_address",
                    details: e.to_string(),
                }
            })?,
            resource_type: ResourceType::from_str(&self.resource_type).map_err(|e| {
                WalletStorageError::DecodingError {
                    operation: "try_into_vault",
                    item: "vault.resource_type",
                    details: e.to_string(),
                }
            })?,
            token_symbol: self.token_symbol,
            revealed_balance: self
                .revealed_balance
                .parse()
                .map_err(|e| WalletStorageError::DecodingError {
                    operation: "try_into_vault",
                    item: "vault.revealed_balance",
                    details: format!("Invalid revealed balance: {}", e),
                })?,
            locked_revealed_balance,
            confidential_balance: self
                .confidential_balance
                .parse()
                .map_err(|e| WalletStorageError::DecodingError {
                    operation: "try_into_vault",
                    item: "vault.confidential_balance",
                    details: format!("Invalid confidential balance: {}", e),
                })?,
            divisibility: u8::try_from(self.divisibility as u32).map_err(|e| WalletStorageError::DecodingError {
                operation: "try_into_vault",
                item: "vault.divisibility",
                details: format!("Invalid divisibility: {}", e),
            })?,
        })
    }
}
