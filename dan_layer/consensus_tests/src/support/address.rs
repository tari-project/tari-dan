//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::fmt::Display;

use serde::{Deserialize, Serialize};
use tari_dan_common_types::NodeAddressable;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TestAddress(pub String);

impl TestAddress {
    pub fn new<T: Into<String>>(s: T) -> Self {
        TestAddress(s.into())
    }
}

impl NodeAddressable for TestAddress {
    fn zero() -> Self {
        TestAddress::new("")
    }

    fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }

    fn from_bytes(bytes: &[u8]) -> Option<Self> {
        std::str::from_utf8(bytes).ok().map(TestAddress::new)
    }
}

impl Display for TestAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TestAddress({})", self.0)
    }
}
