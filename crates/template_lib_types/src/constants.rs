//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! A collection of convenient constant values

use crate::{
    ObjectKey,
    substates::{ComponentAddress, ResourceAddress, VaultId},
};
// TODO: These addresses are set pretty arbitrarily.

/// Resource address for all public identity-based non-fungible tokens.
/// This resource provides a space for a virtual token representing ownership based on a public key.
/// resource_0100000000000000000000000000000000000000000000000000000000000000
pub const PUBLIC_IDENTITY_RESOURCE_ADDRESS: ResourceAddress = ResourceAddress::new(ObjectKey::from_array([
    1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
]));

/// Resource address of the virtual badge that names the component a frame is executing on behalf of.
/// A frame pushed by a component method call carries the badge
/// `nft_<CALLER_COMPONENT_RESOURCE_ADDRESS>_uuid_<caller component address>` in its authorization scope, so
/// `rule!(caller_component(addr))` — and equivalently `require(NonFungibleAddress)` of that badge — is
/// re-checkable at every auth point for the lifetime of the frame.
///
/// `rule!(any_caller_component)` requires this resource rather than a specific badge, and so reads as "called by
/// some component": satisfied by any component caller and by no top-level instruction. Template code should use
/// that spelling rather than naming this address.
///
/// No resource exists at this address and none may be created: the badge is issued by the engine at frame push
/// and cannot be minted, held in a vault, or captured as a `Proof`.
/// resource_0100000000000000000000000000000000000000000000000000000000000001
pub const CALLER_COMPONENT_RESOURCE_ADDRESS: ResourceAddress = ResourceAddress::new(ObjectKey::from_array([
    1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1,
]));

/// Resource address of the virtual badge that names the template of a frame's immediate caller. Stamped
/// alongside [`CALLER_COMPONENT_RESOURCE_ADDRESS`] at every frame push, and the only caller badge a static
/// function call yields (a function frame has no component identity). Since every frame below the top level
/// carries one, `rule!(any_caller_template)` reads as "reached from template code rather than directly from a
/// transaction instruction".
///
/// Subject to the same restrictions as [`CALLER_COMPONENT_RESOURCE_ADDRESS`].
/// resource_0100000000000000000000000000000000000000000000000000000000000002
pub const DIRECT_CALLER_TEMPLATE_RESOURCE_ADDRESS: ResourceAddress = ResourceAddress::new(ObjectKey::from_array([
    1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2,
]));

/// The Tari network native resource address. This token is used for paying network fees, among other things. It is a
/// fungible resource with a divisibility of 6, meaning that the smallest unit is 0.000001 TARI.
/// resource_0101010101010101010101010101010101010101010101010101010101010101
pub const STEALTH_TARI_RESOURCE_ADDRESS: ResourceAddress =
    ResourceAddress::new(ObjectKey::from_array([1u8; ObjectKey::LENGTH]));

/// Shorthand version of the `STEALTH_TARI_RESOURCE_ADDRESS` constant
pub const TARI_TOKEN: ResourceAddress = STEALTH_TARI_RESOURCE_ADDRESS;
#[deprecated(since = "0.24.5", note = "Use TARI_TOKEN instead")]
pub const XTR: ResourceAddress = STEALTH_TARI_RESOURCE_ADDRESS;
/// One XTR in the smallest divisible units i.e. 1 TARI = 1,000,000 micro TARI
/// For example: 10 * TARI = 10 TARI = 10,000,000 micro TARI
pub const TARI: u64 = 1_000_000;
#[deprecated(since = "0.24.5", note = "Use TARI instead")]
pub const ONE_XTR: u64 = TARI;

/// Address of testnet faucet component
pub const XTR_FAUCET_COMPONENT_ADDRESS: ComponentAddress = ComponentAddress::new(ObjectKey::from_array([
    1, 2, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
]));

/// Address of the faucet vault
pub const XTR_FAUCET_VAULT_ADDRESS: VaultId = VaultId::new(ObjectKey::from_array([
    1, 2, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1,
]));

/// Address of the NFT resource used by the XTR faucet to track which public keys have already claimed.
/// Each claim mints an NFT with ID = claimant's public key, then immediately burns it.
/// The burned substate key persists, preventing re-claims.
/// resource_0102030000000000000000000000000000000000000000000000000000000002
pub const XTR_FAUCET_CLAIM_RESOURCE_ADDRESS: ResourceAddress = ResourceAddress::new(ObjectKey::from_array([
    1, 2, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2,
]));

/// The fixed amount dispensed per faucet claim: 1,000 TARI in microtari.
/// Matches the on-chain `FAUCET_AMOUNT` constant in the faucet template.
pub const XTR_FAUCET_AMOUNT: u64 = 1_000 * TARI;

/// Address of the NFT faucet component
pub const NFT_FAUCET_COMPONENT_ADDRESS: ComponentAddress = ComponentAddress::new(ObjectKey::from_array([
    0xff, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
]));
/// Address of the builtin NFT faucet resource
/// resource_ff00000000000000000000000000000000000000000000000000000000000001
pub const NFT_FAUCET_RESOURCE_ADDRESS: ResourceAddress = ResourceAddress::new(ObjectKey::from_array([
    0xff, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1,
]));

/// Metadata key used as convention to represent the symbol (a.k.a. ticker) of a token. Meant as a shorthand,
/// user-friendly identification of the underlying token
pub const TOKEN_SYMBOL: &str = "SYMBOL";
/// Metadata key used as convention to represent the image URL of a token. Meant to be used in user interfaces
/// to display the token's logo or image
pub const IMAGE_URL: &str = "IMAGE_URL";
/// Default divisibility for fungible resources (8)
pub const DEFAULT_DIVISIBILITY: u8 = 8;
