//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_engine_types::substate::SubstateId;
use tari_template_lib::{
    models::{Amount, ResourceAddress},
    resource::ResourceType,
};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VaultModel {
    pub account_address: SubstateId,
    pub address: SubstateId,
    pub resource_address: ResourceAddress,
    pub resource_type: ResourceType,
    pub confidential_balance: Amount,
    pub revealed_balance: Amount,
    pub token_symbol: Option<String>,
}

impl VaultModel {
    pub fn total_balance(&self) -> Amount {
        self.confidential_balance + self.revealed_balance
    }
}

#[derive(Debug, Clone)]
pub struct VaultBalance {
    pub account: SubstateId,
    pub confidential: Amount,
    pub revealed: Amount,
}
