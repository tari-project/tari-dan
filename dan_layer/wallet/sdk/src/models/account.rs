//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::fmt::{Display, Formatter};

use tari_engine_types::substate::SubstateId;
#[cfg(feature = "ts")]
use ts_rs::TS;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct Account {
    pub name: Option<String>,
    pub address: SubstateId,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
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
