//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{fmt::Display, str::FromStr};

use serde::{Deserialize, Serialize};
use tari_common_types::types::FixedHash;
use tari_engine_types::{serde_with, substate::SubstateAddress, TemplateAddress};
use tari_transaction::SubstateRequirement;

#[derive(Debug, Clone)]
pub struct SubstateModel {
    pub module_name: Option<String>,
    pub address: VersionedSubstateAddress,
    pub parent_address: Option<SubstateAddress>,
    pub transaction_hash: FixedHash,
    pub template_address: Option<TemplateAddress>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VersionedSubstateAddress {
    #[serde(with = "serde_with::string")]
    pub address: SubstateAddress,
    pub version: u32,
}

impl FromStr for VersionedSubstateAddress {
    type Err = VersionedSubstateAddressParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.split(':');
        let address = parts
            .next()
            .ok_or_else(|| VersionedSubstateAddressParseError(s.to_string()))?;
        let address =
            SubstateAddress::from_str(address).map_err(|_| VersionedSubstateAddressParseError(s.to_string()))?;
        let version = parts
            .next()
            .map(|v| v.parse())
            .transpose()
            .map_err(|_| VersionedSubstateAddressParseError(s.to_string()))?;
        Ok(Self {
            address,
            version: version.unwrap_or(0),
        })
    }
}

impl Display for VersionedSubstateAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.address, self.version)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("Failed to parse versioned substate address {0}")]
pub struct VersionedSubstateAddressParseError(String);

impl From<VersionedSubstateAddress> for SubstateRequirement {
    fn from(value: VersionedSubstateAddress) -> Self {
        Self::new(value.address, Some(value.version))
    }
}
