//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{fmt::Display, str::FromStr};

use serde::{Deserialize, Serialize};
use tari_common_types::types::FixedHash;
use tari_engine_types::{serde_with, substate::SubstateId, TemplateAddress};
use tari_transaction::SubstateRequirement;

#[derive(Debug, Clone)]
pub struct SubstateModel {
    pub module_name: Option<String>,
    pub address: VersionedSubstateId,
    pub parent_address: Option<SubstateId>,
    pub transaction_hash: FixedHash,
    pub template_address: Option<TemplateAddress>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VersionedSubstateId {
    #[serde(with = "serde_with::string")]
    pub substate_id: SubstateId,
    pub version: u32,
}

impl FromStr for VersionedSubstateId {
    type Err = VersionedSubstateIdParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.split(':');
        let address = parts
            .next()
            .ok_or_else(|| VersionedSubstateIdParseError(s.to_string()))?;
        let address = SubstateId::from_str(address).map_err(|_| VersionedSubstateIdParseError(s.to_string()))?;
        let version = parts
            .next()
            .map(|v| v.parse())
            .transpose()
            .map_err(|_| VersionedSubstateIdParseError(s.to_string()))?;
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

#[derive(Debug, thiserror::Error)]
#[error("Failed to parse versioned substate ID {0}")]
pub struct VersionedSubstateIdParseError(String);

impl From<VersionedSubstateId> for SubstateRequirement {
    fn from(value: VersionedSubstateId) -> Self {
        Self::new(value.substate_id, Some(value.version))
    }
}
