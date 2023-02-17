//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use crate::{models::ResourceAddress, Hash};

// TODO: This is set pretty arbitrarily.
/// Resource address for all public identity-based non-fungible tokens.
/// This resource provides a space for a virtual token representing ownership based on a public key.
pub const PUBLIC_IDENTITY_RESOURCE: ResourceAddress = ResourceAddress::new(Hash::from_array([
    1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
]));

pub const SECOND_LAYER_TARI_RESOURCE: ResourceAddress = ResourceAddress::new(Hash::from_array([1u8; 32]));
