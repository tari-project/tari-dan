//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{fmt, str::FromStr};

use tari_bor::{Deserialize, Serialize};
use tari_engine_types::substate::SubstateId;

use crate::{SubstateAddress, ToSubstateAddress, VersionedSubstateId};

pub trait LockIntent {
    fn substate_id(&self) -> &SubstateId;
    fn lock_type(&self) -> SubstateLockType;
    fn version_to_lock(&self) -> u32;
    fn requested_version(&self) -> Option<u32>;

    fn to_versioned_substate_id(&self) -> VersionedSubstateId {
        VersionedSubstateId::new(self.substate_id().clone(), self.version_to_lock())
    }
}

impl<T: LockIntent> ToSubstateAddress for T {
    fn to_substate_address(&self) -> SubstateAddress {
        SubstateAddress::from_substate_id(self.substate_id(), self.version_to_lock())
    }
}

/// Substate lock flags
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "../../bindings/src/types/")
)]
pub enum SubstateLockType {
    Read,
    Write,
    Output,
}

impl SubstateLockType {
    pub fn is_write(&self) -> bool {
        matches!(self, Self::Write)
    }

    pub fn is_read(&self) -> bool {
        matches!(self, Self::Read)
    }

    pub fn is_output(&self) -> bool {
        matches!(self, Self::Output)
    }
}

impl fmt::Display for SubstateLockType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read => write!(f, "Read"),
            Self::Write => write!(f, "Write"),
            Self::Output => write!(f, "Output"),
        }
    }
}

impl FromStr for SubstateLockType {
    type Err = SubstateLockFlagParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Read" => Ok(Self::Read),
            "Write" => Ok(Self::Write),
            "Output" => Ok(Self::Output),
            _ => Err(SubstateLockFlagParseError),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("Failed to parse SubstateLockFlag")]
pub struct SubstateLockFlagParseError;
