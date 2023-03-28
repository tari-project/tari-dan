//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::str::FromStr;

use chrono::NaiveDateTime;
use diesel::{Identifiable, Queryable};
use tari_dan_wallet_sdk::storage::WalletStorageError;
use tari_engine_types::substate::SubstateAddress;
use tari_template_lib::{
    models::{Amount, ResourceAddress},
    resource::ResourceType,
};

use crate::schema::vaults;

#[derive(Debug, Clone, Queryable, Identifiable)]
#[diesel(table_name = vaults)]
pub struct Vault {
    pub id: i32,
    pub account_id: i32,
    pub address: String,
    pub resource_address: String,
    pub resource_type: String,
    pub balance: i64,
    pub token_symbol: Option<String>,
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
            resource_type: db_str_to_resource_type(&self.resource_type)?,
            token_symbol: self.token_symbol,
            balance: Amount::new(self.balance),
        })
    }
}

fn db_str_to_resource_type(s: &str) -> Result<ResourceType, WalletStorageError> {
    match s {
        "Fungible" => Ok(ResourceType::Fungible),
        "NonFungible" => Ok(ResourceType::NonFungible),
        "Confidential" => Ok(ResourceType::Confidential),
        _ => Err(WalletStorageError::DecodingError {
            operation: "db_str_to_resource_type",
            item: "vault.resource_type",
            details: format!("Invalid resource type: {}", s),
        }),
    }
}
