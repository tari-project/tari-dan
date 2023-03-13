//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::str::FromStr;

use chrono::NaiveDateTime;
use diesel::{Identifiable, Queryable};
use tari_dan_wallet_sdk::storage::WalletStorageError;
use tari_engine_types::substate::SubstateAddress;
use tari_template_lib::models::{Amount, ResourceAddress};

use crate::schema::vaults;

#[derive(Debug, Clone, Queryable, Identifiable)]
#[diesel(table_name = vaults)]
pub struct Vault {
    pub id: i32,
    pub account_id: i32,
    pub address: String,
    pub resource_address: String,
    pub balance: i64,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

impl Vault {
    pub(crate) fn try_into_vault(
        self,
        account_address: SubstateAddress,
    ) -> Result<tari_dan_wallet_sdk::models::VaultModel, WalletStorageError> {
        Ok(tari_dan_wallet_sdk::models::VaultModel {
            account_address,
            address: SubstateAddress::from_str(&self.address).map_err(|e| WalletStorageError::DecodingError {
                operation: "try_into_vault",
                item: "vault.address",
                details: e.to_string(),
            })?,
            resource_address: ResourceAddress::from_str(&self.resource_address).map_err(|e| {
                WalletStorageError::DecodingError {
                    operation: "try_into_vault",
                    item: "vault.resource_address",
                    details: e.to_string(),
                }
            })?,
            balance: Amount(self.balance),
        })
    }
}
