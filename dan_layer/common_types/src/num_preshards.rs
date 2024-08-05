//    Copyright 2024 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use std::{error::Error, fmt::Display};

use serde::{Deserialize, Serialize};

#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "../../bindings/src/types/")
)]
#[derive(Clone, Debug, Copy, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub enum NumPreshards {
    P1 = 1,
    P2 = 2,
    P4 = 4,
    P8 = 8,
    P16 = 16,
    P32 = 32,
    P64 = 64,
    P128 = 128,
    P256 = 256,
}

impl NumPreshards {
    pub const MAX: Self = Self::P256;

    pub fn as_u32(self) -> u32 {
        self as u32
    }

    pub fn is_one(self) -> bool {
        self == Self::P1
    }
}

impl TryFrom<u32> for NumPreshards {
    type Error = InvalidNumPreshards;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::P1),
            2 => Ok(Self::P2),
            4 => Ok(Self::P4),
            8 => Ok(Self::P8),
            16 => Ok(Self::P16),
            32 => Ok(Self::P32),
            64 => Ok(Self::P64),
            128 => Ok(Self::P128),
            256 => Ok(Self::P256),
            _ => Err(InvalidNumPreshards(value)),
        }
    }
}

impl From<NumPreshards> for u32 {
    fn from(num_preshards: NumPreshards) -> u32 {
        num_preshards.as_u32()
    }
}

impl Display for NumPreshards {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[derive(Debug)]
pub struct InvalidNumPreshards(u32);

impl Error for InvalidNumPreshards {}

impl Display for InvalidNumPreshards {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} is not a valid number of pre-shards", self.0)
    }
}
