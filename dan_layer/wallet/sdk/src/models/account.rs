//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::fmt::{Display, Formatter};

use tari_engine_types::substate::SubstateAddress;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Account {
    pub name: Option<String>,
    pub address: SubstateAddress,
    pub key_index: u64,
    pub is_default: bool,
}

impl Display for Account {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self.name {
            Some(ref name) => write!(f, "{} ({})", name, self.address),
            None => write!(f, "{}", self.address),
        }
    }
}
