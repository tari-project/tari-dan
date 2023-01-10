//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::str::FromStr;

use anyhow::anyhow;
use serde::{Deserialize, Serialize};
use tari_dan_common_types::serde_with;
use tari_engine_types::substate::SubstateAddress;

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub struct VersionedSubstateAddress {
    #[serde(with = "serde_with::string")]
    pub address: SubstateAddress,
    pub version: u32,
}

impl FromStr for VersionedSubstateAddress {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.split(':');
        let address = parts.next().ok_or_else(|| anyhow!("Invalid substate address"))?;
        let address = SubstateAddress::from_str(address)?;
        let version = parts.next().map(|v| v.parse()).transpose()?;
        Ok(Self {
            address,
            version: version.unwrap_or(0),
        })
    }
}
