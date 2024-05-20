//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::fmt::Display;

use tari_bor::{Deserialize, Serialize};
#[cfg(feature = "ts")]
use ts_rs::TS;

pub type LockId = u32;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub enum LockFlag {
    Read = 0x01,
    Write = 0x02,
}

impl LockFlag {
    pub fn is_read(&self) -> bool {
        matches!(self, Self::Read)
    }

    pub fn is_write(&self) -> bool {
        matches!(self, Self::Write)
    }
}

impl Display for LockFlag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LockFlag::Read => write!(f, "Read"),
            LockFlag::Write => write!(f, "Write"),
        }
    }
}
