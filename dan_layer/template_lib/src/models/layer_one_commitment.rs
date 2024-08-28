//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use std::{
    fmt::{Display, Formatter},
    str::FromStr,
};

use serde::{Deserialize, Serialize};
use tari_bor::BorTag;

use crate::models::{BinaryTag, KeyParseError, ObjectKey};

const TAG: u64 = BinaryTag::UnclaimedConfidentialOutputAddress.as_u64();

/// The unique identification of a unclaimed confidential output in the Tari network.
/// Used when a user wants to claim burned funds from the Minotari network into the Tari network
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "../../bindings/src/types/")
)]
#[serde(transparent)]
pub struct UnclaimedConfidentialOutputAddress(#[cfg_attr(feature = "ts", ts(type = "string"))] BorTag<ObjectKey, TAG>);

impl UnclaimedConfidentialOutputAddress {
    pub fn from_hex(hex: &str) -> Result<Self, KeyParseError> {
        Ok(Self(BorTag::new(ObjectKey::from_hex(hex)?)))
    }

    pub fn try_from_commitment(commitment_bytes: &[u8]) -> Result<Self, KeyParseError> {
        Ok(Self(BorTag::new(ObjectKey::try_from(commitment_bytes)?)))
    }

    pub fn as_object_key(&self) -> &ObjectKey {
        &self.0
    }

    pub fn to_vec(&self) -> Vec<u8> {
        self.0.to_vec()
    }
}

impl From<ObjectKey> for UnclaimedConfidentialOutputAddress {
    fn from(key: ObjectKey) -> Self {
        Self(BorTag::new(key))
    }
}

impl TryFrom<&[u8]> for UnclaimedConfidentialOutputAddress {
    type Error = KeyParseError;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        Ok(Self(BorTag::new(ObjectKey::try_from(value)?)))
    }
}

impl Display for UnclaimedConfidentialOutputAddress {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "commitment_{}", self.0.inner())
    }
}

impl FromStr for UnclaimedConfidentialOutputAddress {
    type Err = KeyParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.strip_prefix("commitment_").unwrap_or(s);
        Self::from_hex(s)
    }
}
