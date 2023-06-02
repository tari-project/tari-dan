//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::BTreeMap, str::FromStr};

use chrono::NaiveDateTime;
use tari_engine_types::substate::SubstateAddress;
use tari_template_lib::prelude::{Metadata, NonFungibleId, ResourceAddress};

use crate::schema::non_fungible_tokens;
use tari_dan_wallet_sdk::storage::WalletStorageError;

#[derive(Debug, Clone, Identifiable, Queryable)]
#[diesel(table_name = non_fungible_tokens)]
pub struct NonFungibleToken {
    pub id: i32,
    pub account_id: i32,
    pub account_address: String,
    pub resource_address: String,
    pub token_symbol: String,
    pub nft_id: String,
    pub metadata: String,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

impl TryFrom<NonFungibleToken> for tari_dan_wallet_sdk::models::NonFungibleToken {
    type Error = WalletStorageError;

    fn try_from(non_fungible: NonFungibleToken) -> Result<Self, Self::Error> {
        let metadata: BTreeMap<String, String> =
            serde_json::from_str(&non_fungible.metadata).map_err(|e| WalletStorageError::DecodingError {
                operation: "try_from",
                item: "non_fungible_tokens.metadata",
                details: e.to_string(),
            })?;
        Ok(Self {
            account_address: SubstateAddress::from_str(&non_fungible.account_address).map_err(|e| {
                WalletStorageError::DecodingError {
                    operation: "try_from",
                    item: "non_fungible_tokens.address",
                    details: e.to_string(),
                }
            })?,
            metadata: Metadata::from(metadata),
            resource_address: ResourceAddress::from_str(&non_fungible.resource_address).map_err(|e| {
                WalletStorageError::DecodingError {
                    operation: "try_from",
                    item: "non_fungible_tokens.resource_address",
                    details: e.to_string(),
                }
            })?,
            nft_id: NonFungibleId::try_from_canonical_string(&non_fungible.nft_id).map_err(|e| {
                WalletStorageError::DecodingError {
                    operation: "try_from",
                    item: "non_fungible_tokens.nft_id",
                    details: format!("{:?}", e),
                }
            })?,
            token_symbol: non_fungible.token_symbol,
        })
    }
}
