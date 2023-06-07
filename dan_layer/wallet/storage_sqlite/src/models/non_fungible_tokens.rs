//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::BTreeMap;

use chrono::NaiveDateTime;
use tari_dan_wallet_sdk::storage::WalletStorageError;
use tari_template_lib::{
    models::VaultId,
    prelude::{Metadata, NonFungibleId},
};

use crate::schema::non_fungible_tokens;

#[derive(Debug, Clone, Identifiable, Queryable)]
#[diesel(table_name = non_fungible_tokens)]
pub struct NonFungibleToken {
    pub id: i32,
    pub vault_id: i32,
    pub nft_id: String,
    pub metadata: String,
    pub is_burned: bool,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

impl NonFungibleToken {
    pub fn try_into_non_fungible_token(
        self,
        vault_id: VaultId,
    ) -> Result<tari_dan_wallet_sdk::models::NonFungibleToken, WalletStorageError> {
        let metadata: BTreeMap<String, String> =
            serde_json::from_str(&self.metadata).map_err(|e| WalletStorageError::DecodingError {
                operation: "try_from",
                item: "non_fungible_tokens.metadata",
                details: e.to_string(),
            })?;
        Ok(tari_dan_wallet_sdk::models::NonFungibleToken {
            metadata: Metadata::from(metadata),
            nft_id: NonFungibleId::try_from_canonical_string(&self.nft_id).map_err(|e| {
                WalletStorageError::DecodingError {
                    operation: "try_from",
                    item: "non_fungible_tokens.nft_id",
                    details: format!("{:?}", e),
                }
            })?,
            vault_id,
            is_burned: self.is_burned,
        })
    }
}
