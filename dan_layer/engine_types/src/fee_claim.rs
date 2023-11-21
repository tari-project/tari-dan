//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use std::{fmt, fmt::Display};

use serde::{Deserialize, Serialize};
use tari_bor::BorTag;
use tari_common_types::types::PublicKey;
use tari_template_lib::{models::BinaryTag, prelude::Amount, Hash};

use crate::hashing::{hasher32, EngineHashDomainLabel};

const TAG: u64 = BinaryTag::FeeClaim.as_u64();

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct FeeClaimAddress(BorTag<Hash, TAG>);

impl FeeClaimAddress {
    pub const fn new(address: Hash) -> Self {
        Self(BorTag::new(address))
    }

    pub fn from_addr<TAddr: AsRef<[u8]>>(epoch: u64, addr: TAddr) -> Self {
        let hash = hasher32(EngineHashDomainLabel::FeeClaimAddress)
            .chain(&epoch)
            .chain(addr.as_ref())
            .result();
        Self::new(hash)
    }

    pub fn hash(&self) -> &Hash {
        self.0.inner()
    }
}

impl<T: Into<Hash>> From<T> for FeeClaimAddress {
    fn from(address: T) -> Self {
        Self::new(address.into())
    }
}

impl Display for FeeClaimAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "feeclaim_{}", self.hash())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeeClaim {
    pub epoch: u64,
    pub validator_public_key: PublicKey,
    pub amount: Amount,
}
