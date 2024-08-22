//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::fmt::{Display, Formatter};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "../../bindings/src/types/")
)]
pub struct LeaderFee {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub fee: u64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub global_exhaust_burn: u64,
}

impl LeaderFee {
    pub fn fee(&self) -> u64 {
        self.fee
    }

    pub fn global_exhaust_burn(&self) -> u64 {
        self.global_exhaust_burn
    }
}

impl Display for LeaderFee {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "Leader fee: {}, Burnt: {}", self.fee, self.global_exhaust_burn)
    }
}
