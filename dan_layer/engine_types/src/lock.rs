//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::fmt::Display;

pub type LockId = u32;
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub enum LockFlag {
    Read,
    Write,
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
