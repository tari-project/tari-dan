//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    fmt::{Debug, Display},
    hash::Hash,
};

use tari_common_types::types::PublicKey;
use tari_utilities::ByteArray;

pub trait NodeAddressable: Eq + Hash + Clone + Debug + Send + Sync + Display {
    fn zero() -> Self;
    fn as_bytes(&self) -> &[u8];
}

impl NodeAddressable for String {
    fn zero() -> Self {
        "".to_string()
    }

    fn as_bytes(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl NodeAddressable for &str {
    fn zero() -> Self {
        ""
    }

    fn as_bytes(&self) -> &[u8] {
        str::as_bytes(self)
    }
}

impl NodeAddressable for PublicKey {
    fn zero() -> Self {
        PublicKey::default()
    }

    fn as_bytes(&self) -> &[u8] {
        <PublicKey as ByteArray>::as_bytes(self)
    }
}
