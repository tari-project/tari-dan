//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    fmt::{Debug, Display},
    hash::Hash,
};

use serde::{de::DeserializeOwned, Serialize};
use tari_common_types::types::PublicKey;

pub trait NodeAddressable:
    Eq + Hash + Clone + Debug + Ord + Send + Sync + Display + Serialize + DeserializeOwned
{
    fn zero() -> Self;

    fn try_from_public_key(_: &PublicKey) -> Option<Self> {
        None
    }
}

impl NodeAddressable for String {
    fn zero() -> Self {
        "".to_string()
    }

    fn try_from_public_key(_: &PublicKey) -> Option<Self> {
        None
    }
}

impl NodeAddressable for PublicKey {
    fn zero() -> Self {
        PublicKey::default()
    }

    fn try_from_public_key(public_key: &PublicKey) -> Option<Self> {
        Some(public_key.clone())
    }
}

pub trait DerivableFromPublicKey: NodeAddressable {
    fn derive_from_public_key(public_key: &PublicKey) -> Self {
        Self::try_from_public_key(public_key)
            .expect("Marker trait DerivableFromPublicKey must always return Some from try_from_public_key")
    }

    fn eq_to_public_key(&self, public_key: &PublicKey) -> bool {
        *self == Self::derive_from_public_key(public_key)
    }
}

impl DerivableFromPublicKey for PublicKey {}
