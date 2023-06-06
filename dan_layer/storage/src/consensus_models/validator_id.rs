//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::fmt::Display;

use serde::{Deserialize, Serialize};
use tari_common_types::types::FixedHashSizeError;
use tari_dan_common_types::{NodeAddressable, ShardId};

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
pub struct ValidatorId(ShardId);

impl ValidatorId {
    pub const fn zero() -> Self {
        ValidatorId(ShardId::zero())
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }

    pub fn shard_id(&self) -> ShardId {
        self.0
    }
}

impl AsRef<[u8]> for ValidatorId {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl Display for ValidatorId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for byte in self.as_bytes() {
            write!(f, "{:02x}", byte)?;
        }
        Ok(())
    }
}

impl NodeAddressable for ValidatorId {
    fn zero() -> Self {
        ValidatorId::zero()
    }

    fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

impl TryFrom<Vec<u8>> for ValidatorId {
    type Error = FixedHashSizeError;

    fn try_from(value: Vec<u8>) -> Result<Self, Self::Error> {
        Ok(ValidatorId(ShardId::try_from(value)?))
    }
}
