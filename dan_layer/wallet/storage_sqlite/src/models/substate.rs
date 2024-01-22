//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use chrono::NaiveDateTime;
use diesel::{Identifiable, Queryable};
use tari_common_types::types::FixedHash;
use tari_dan_wallet_sdk::{
    models::{SubstateModel, VersionedSubstateId},
    storage::WalletStorageError,
};
use tari_template_lib::Hash;
use tari_utilities::hex::Hex;

use crate::schema::substates;

#[derive(Debug, Clone, Queryable, Identifiable)]
#[diesel(table_name = substates)]
pub struct Substate {
    pub id: i32,
    pub module_name: Option<String>,
    pub address: String,
    pub parent_address: Option<String>,
    pub version: i32,
    pub transaction_hash: String,
    pub template_address: Option<String>,
    pub created_at: NaiveDateTime,
}

impl Substate {
    pub fn try_to_record(&self) -> Result<SubstateModel, WalletStorageError> {
        Ok(SubstateModel {
            module_name: self.module_name.clone(),
            address: VersionedSubstateId {
                substate_id: self.address.parse().unwrap(),
                version: self.version as u32,
            },
            parent_address: self.parent_address.as_ref().map(|s| s.parse().unwrap()),
            transaction_hash: FixedHash::from_hex(&self.transaction_hash).map_err(|e| {
                WalletStorageError::DecodingError {
                    operation: "try_to_record",
                    item: "transaction_hash",
                    details: e.to_string(),
                }
            })?,
            template_address: self
                .template_address
                .as_ref()
                .map(|s| Hash::from_hex(s))
                .transpose()
                .map_err(|e| WalletStorageError::DecodingError {
                    operation: "try_to_record",
                    item: "template_address",
                    details: e.to_string(),
                })?,
        })
    }
}
