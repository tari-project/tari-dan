//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::str::FromStr;

use diesel::{Identifiable, Queryable};
use tari_engine_types::substate::SubstateId;
use tari_ootle_common_types::VersionedSubstateId;
use tari_ootle_wallet_sdk::{models::SubstateModel, storage::WalletStorageError};
use tari_template_lib_types::Hash32;
use time::PrimitiveDateTime;

use crate::{schema::substates, serialization::deserialize_json};

#[derive(Debug, Clone, Queryable, Identifiable)]
#[diesel(table_name = substates)]
pub struct Substate {
    pub id: i32,
    pub module_name: Option<String>,
    pub address: String,
    pub parent_address: Option<String>,
    pub referenced_substates: String,
    pub template_address: Option<String>,
    pub created_at: PrimitiveDateTime,
    pub version: i64,
}

impl Substate {
    pub fn try_to_record(&self) -> Result<SubstateModel, WalletStorageError> {
        Ok(SubstateModel {
            module_name: self.module_name.clone(),
            substate_id: VersionedSubstateId::new(SubstateId::from_str(&self.address).unwrap(), self.version as u64),
            parent_address: self.parent_address.as_ref().map(|s| s.parse().unwrap()),
            referenced_substates: deserialize_json(&self.referenced_substates)?,
            template_address: self
                .template_address
                .as_ref()
                .map(|s| Hash32::from_hex(s))
                .transpose()
                .map_err(|e| WalletStorageError::DecodingError {
                    operation: "try_to_record",
                    item: "template_address",
                    details: e.to_string(),
                })?,
        })
    }
}
