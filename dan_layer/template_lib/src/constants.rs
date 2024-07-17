//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! A collection of convenient constant values

use crate::models::{ComponentAddress, ObjectKey, ResourceAddress, VaultId};

// TODO: This is set pretty arbitrarily.

/// Resource address for all public identity-based non-fungible tokens.
/// This resource provides a space for a virtual token representing ownership based on a public key.
pub const PUBLIC_IDENTITY_RESOURCE_ADDRESS: ResourceAddress = ResourceAddress::new(ObjectKey::from_array([
    1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
]));

/// The Tari network native resource address, used for paying network fees
pub const CONFIDENTIAL_TARI_RESOURCE_ADDRESS: ResourceAddress =
    ResourceAddress::new(ObjectKey::from_array([1u8; ObjectKey::LENGTH]));

/// Shorthand version of the `CONFIDENTIAL_TARI_RESOURCE_ADDRESS` constant
pub const XTR: ResourceAddress = CONFIDENTIAL_TARI_RESOURCE_ADDRESS;

/// Address of testnet faucet component
pub const XTR_FAUCET_COMPONENT_ADDRESS: ComponentAddress = ComponentAddress::new(ObjectKey::from_array([
    1, 2, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
]));

/// Address of the faucet vault
pub const XTR_FAUCET_VAULT_ADDRESS: VaultId = VaultId::new(ObjectKey::from_array([
    1, 2, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1,
]));
