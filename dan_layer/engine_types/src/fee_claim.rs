//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use std::{fmt, fmt::Display, str::FromStr};

use serde::{Deserialize, Serialize};
use tari_bor::BorTag;
use tari_common_types::types::PublicKey;
use tari_template_lib::{
    models::{BinaryTag, KeyParseError, ObjectKey},
    prelude::Amount,
    Hash,
};
#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::hashing::{hasher32, EngineHashDomainLabel};

const TAG: u64 = BinaryTag::FeeClaim.as_u64();

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct FeeClaimAddress(#[cfg_attr(feature = "ts", ts(type = "string"))] BorTag<ObjectKey, TAG>);

impl FeeClaimAddress {
    pub const fn from_hash(hash: Hash) -> Self {
        let key = ObjectKey::from_array(hash.into_array());
        Self(BorTag::new(key))
    }

    pub fn from_addr<TAddr: AsRef<[u8]>>(epoch: u64, addr: TAddr) -> Self {
        let hash = hasher32(EngineHashDomainLabel::FeeClaimAddress)
            .chain(&epoch)
            .chain(addr.as_ref())
            .result();
        Self::from_hash(hash)
    }

    pub fn as_object_key(&self) -> &ObjectKey {
        self.0.inner()
    }

    pub fn from_hex(hex: &str) -> Result<Self, KeyParseError> {
        Ok(Self(BorTag::new(ObjectKey::from_hex(hex)?)))
    }
}

impl<T: Into<Hash>> From<T> for FeeClaimAddress {
    fn from(address: T) -> Self {
        Self::from_hash(address.into())
    }
}

impl Display for FeeClaimAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "feeclaim_{}", self.as_object_key())
    }
}

impl FromStr for FeeClaimAddress {
    type Err = KeyParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.strip_prefix("feeclaim_").unwrap_or(s);
        Self::from_hex(s)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct FeeClaim {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub epoch: u64,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub validator_public_key: PublicKey,
    pub amount: Amount,
}
