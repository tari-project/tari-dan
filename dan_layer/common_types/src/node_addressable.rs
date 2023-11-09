//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    fmt::{Debug, Display},
    hash::Hash,
};

use serde::{de::DeserializeOwned, Serialize};
use tari_common_types::types::PublicKey;
use tari_crypto::tari_utilities::ByteArray;

pub trait NodeAddressable:
    Eq + Hash + Clone + Debug + Ord + Send + Sync + Display + Serialize + DeserializeOwned
{
    fn zero() -> Self;
    fn as_bytes(&self) -> &[u8];

    fn from_bytes(bytes: &[u8]) -> Option<Self>;
}

impl NodeAddressable for String {
    fn zero() -> Self {
        "".to_string()
    }

    fn as_bytes(&self) -> &[u8] {
        self.as_bytes()
    }

    fn from_bytes(bytes: &[u8]) -> Option<Self> {
        String::from_utf8(bytes.to_vec()).ok()
    }
}

impl NodeAddressable for PublicKey {
    fn zero() -> Self {
        PublicKey::default()
    }

    fn as_bytes(&self) -> &[u8] {
        <PublicKey as ByteArray>::as_bytes(self)
    }

    fn from_bytes(bytes: &[u8]) -> Option<Self> {
        ByteArray::from_canonical_bytes(bytes).ok()
    }
}
