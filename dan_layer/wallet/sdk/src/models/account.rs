//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_engine_types::substate::SubstateAddress;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Account {
    pub name: String,
    pub address: SubstateAddress,
    pub key_index: u64,
    pub is_default: bool,
}
