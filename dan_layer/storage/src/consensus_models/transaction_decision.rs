//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    fmt,
    fmt::{Display, Formatter},
    str::FromStr,
};

use serde::{Deserialize, Serialize};

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub enum Decision {
    /// Decision to COMMIT the transaction
    Commit,
    /// Decision to ABORT the transaction
    Abort,
}

impl Decision {
    pub fn is_commit(&self) -> bool {
        matches!(self, Decision::Commit)
    }

    pub fn is_abort(&self) -> bool {
        matches!(self, Decision::Abort)
    }

    pub fn as_u8(&self) -> u8 {
        match self {
            Decision::Commit => 0,
            Decision::Abort => 1,
        }
    }

    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Decision::Commit),
            1 => Some(Decision::Abort),
            _ => None,
        }
    }
}

impl Display for Decision {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Decision::Commit => write!(f, "Commit"),
            Decision::Abort => write!(f, "Abort"),
        }
    }
}

impl FromStr for Decision {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Commit" => Ok(Decision::Commit),
            "Abort" => Ok(Decision::Abort),
            _ => Err(()),
        }
    }
}
