//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{fmt::Display, str::FromStr};

use crate::TariSwarmError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TariNetwork {
    DevNet,
}

impl TariNetwork {
    pub const fn as_str(&self) -> &'static str {
        match self {
            TariNetwork::DevNet => "devnet",
        }
    }
}

impl Display for TariNetwork {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for TariNetwork {
    type Err = TariSwarmError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "devnet" => Ok(TariNetwork::DevNet),
            _ => Err(TariSwarmError::ProtocolVersionParseFailed { field: "network" }),
        }
    }
}
