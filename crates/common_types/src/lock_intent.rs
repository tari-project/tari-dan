//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{fmt, str::FromStr};

use borsh::BorshSerialize;
use minicbor::{CborLen, Decode, Encode};
use tari_bor::{Deserialize, Serialize};
use tari_engine_types::substate::SubstateId;

use crate::{SubstateAddress, ToSubstateAddress, VersionedSubstateIdRef};

pub trait LockIntent {
    fn substate_id(&self) -> &SubstateId;
    fn lock_type(&self) -> SubstateLockType;
    fn version_to_lock(&self) -> u64;
    fn requested_version(&self) -> Option<u64>;

    fn to_versioned_substate_id_ref(&self) -> VersionedSubstateIdRef<'_> {
        VersionedSubstateIdRef::new(self.substate_id(), self.version_to_lock())
    }
}

impl<T: LockIntent> ToSubstateAddress for T {
    fn to_substate_address(&self) -> SubstateAddress {
        SubstateAddress::from_substate_id(self.substate_id(), self.version_to_lock())
    }
}

/// Substate lock flags
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, BorshSerialize, Encode, Decode, CborLen)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum SubstateLockType {
    #[n(0)]
    Read,
    #[n(1)]
    Write,
    #[n(2)]
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

    pub fn is_input(&self) -> bool {
        !self.is_output()
    }

    pub fn allows(&self, other: SubstateLockType) -> bool {
        match self {
            Self::Read => matches!(other, Self::Read),
            Self::Write => matches!(other, Self::Read | Self::Write),
            Self::Output => matches!(other, Self::Output),
        }
    }

    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Read => "Read",
            Self::Write => "Write",
            Self::Output => "Output",
        }
    }
}

impl fmt::Display for SubstateLockType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_allows_read_lock_if_write_lock() {
        assert!(SubstateLockType::Write.allows(SubstateLockType::Read));
        assert!(SubstateLockType::Write.allows(SubstateLockType::Write));
        assert!(!SubstateLockType::Write.allows(SubstateLockType::Output));
        assert!(!SubstateLockType::Read.allows(SubstateLockType::Write));
        assert!(SubstateLockType::Read.allows(SubstateLockType::Read));
        assert!(!SubstateLockType::Read.allows(SubstateLockType::Output));
        assert!(!SubstateLockType::Output.allows(SubstateLockType::Read));
        assert!(!SubstateLockType::Output.allows(SubstateLockType::Write));
        assert!(SubstateLockType::Output.allows(SubstateLockType::Output));
    }
}
