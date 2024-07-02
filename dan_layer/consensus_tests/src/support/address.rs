//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::fmt::Display;

use serde::{Deserialize, Serialize};
use tari_common_types::types::PublicKey;
use tari_dan_common_types::{DerivableFromPublicKey, NodeAddressable};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TestAddress(pub String);

impl TestAddress {
    pub fn new<T: Into<String>>(s: T) -> Self {
        TestAddress(s.into())
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

impl NodeAddressable for TestAddress {
    fn zero() -> Self {
        TestAddress::new("")
    }

    fn try_from_public_key(_: &PublicKey) -> Option<Self> {
        None
    }
}

impl Display for TestAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TestAddress({})", self.0)
    }
}

impl DerivableFromPublicKey for TestAddress {
    fn derive_from_public_key(_public_key: &PublicKey) -> Self {
        unreachable!("TestAddress cannot be derived from a public key")
    }

    fn eq_to_public_key(&self, _public_key: &PublicKey) -> bool {
        // Hack to get around the fact that we cannot derive TestAddress from a public key
        // This is used to validate the block proposer, so this check will always pass in tests.
        true
    }
}
