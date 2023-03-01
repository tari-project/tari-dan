//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, Eq, PartialEq)]
pub struct ObjectClaim {}

impl ObjectClaim {
    pub fn is_valid(&self) -> bool {
        // TODO: Implement this
        true
    }
}
