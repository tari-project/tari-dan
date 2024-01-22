//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{fmt::Display, str::FromStr};

use anyhow::anyhow;
use serde::{Deserialize, Serialize};
use tari_dan_common_types::SubstateAddress;
use tari_engine_types::{serde_with, substate::SubstateId};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VersionedSubstateId {
    #[serde(with = "serde_with::string")]
    pub substate_id: SubstateId,
    pub version: u32,
}

impl VersionedSubstateId {
    pub fn to_substate_address(&self) -> SubstateAddress {
        SubstateAddress::from_address(&self.substate_id, self.version)
    }
}

impl FromStr for VersionedSubstateId {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.split(':');
        let address = parts.next().ok_or_else(|| anyhow!("Invalid substate id"))?;
        let address = SubstateId::from_str(address)?;
        let version = parts.next().map(|v| v.parse()).transpose()?;
        Ok(Self {
            substate_id: address,
            version: version.unwrap_or(0),
        })
    }
}

impl Display for VersionedSubstateId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.substate_id, self.version)
    }
}
