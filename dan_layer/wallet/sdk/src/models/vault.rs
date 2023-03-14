//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_engine_types::substate::SubstateAddress;
use tari_template_lib::{
    models::{Amount, ResourceAddress},
    resource::ResourceType,
};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VaultModel {
    pub account_address: SubstateAddress,
    pub address: SubstateAddress,
    pub resource_address: ResourceAddress,
    pub resource_type: ResourceType,
    pub balance: Amount,
}
