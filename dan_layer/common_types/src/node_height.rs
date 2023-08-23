//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    fmt::{Display, Formatter},
    ops::{Add, Sub},
};

use serde::{Deserialize, Serialize};

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize, Serialize)]
pub struct NodeHeight(pub u64);

impl NodeHeight {
    pub const fn as_u64(self) -> u64 {
        self.0
    }

    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }

    pub const fn zero() -> Self {
        Self(0)
    }

    pub const fn to_le_bytes(self) -> [u8; 8] {
        self.0.to_le_bytes()
    }

    pub const fn saturating_sub(self, other: Self) -> Self {
        Self(self.0.saturating_sub(other.0))
    }

    pub const fn checked_sub(self, other: Self) -> Option<Self> {
        // Option::map as a const fn is not yet stablized, so we re-implement it here
        match self.0.checked_sub(other.0) {
            Some(v) => Some(Self(v)),
            None => None,
        }
    }
}

impl Add for NodeHeight {
    type Output = NodeHeight;

    fn add(self, rhs: Self) -> Self::Output {
        NodeHeight(self.0 + rhs.0)
    }
}

impl Sub for NodeHeight {
    type Output = NodeHeight;

    fn sub(self, rhs: Self) -> Self::Output {
        NodeHeight(self.0 - rhs.0)
    }
}

impl From<u64> for NodeHeight {
    fn from(height: u64) -> Self {
        NodeHeight(height)
    }
}

impl Display for NodeHeight {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "NodeHeight({})", self.0)
    }
}
