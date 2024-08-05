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
    One = 1,
    Two = 2,
    Four = 4,
    Eight = 8,
    Sixteen = 16,
    ThirtyTwo = 32,
    SixtyFour = 64,
    OneTwentyEight = 128,
    TwoFiftySix = 256,
}

impl NumPreshards {
    pub const MAX: Self = Self::TwoFiftySix;

    pub fn as_u32(self) -> u32 {
        self as u32
    }

    pub fn is_one(self) -> bool {
        self == Self::One
    }
}

impl TryFrom<u32> for NumPreshards {
    type Error = InvalidNumPreshards;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::One),
            2 => Ok(Self::Two),
            4 => Ok(Self::Four),
            8 => Ok(Self::Eight),
            16 => Ok(Self::Sixteen),
            32 => Ok(Self::ThirtyTwo),
            64 => Ok(Self::SixtyFour),
            128 => Ok(Self::OneTwentyEight),
            256 => Ok(Self::TwoFiftySix),
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
