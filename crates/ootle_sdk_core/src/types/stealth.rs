//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Stealth (confidential-transfer) boundary records.
//!
//! These are the typed records the stealth surface builds on. They follow the exact
//! conventions established in [`crate::types::bytes`]: crypto material crosses as **lowercase-hex
//! byte newtypes** (no `0x` prefix), amounts are plain `u64`/[`BoundaryAmount`], addresses are
//! `…Str`, and **no `tari_*` type leaks** into a boundary record. Every record derives
//! `Debug + Clone + PartialEq + Serialize + Deserialize`.
//!
//! Two families live here:
//!
//! - The developer-facing intent ([`StealthTransferIntent`] + [`StealthInputSpec`] + [`StealthOutputSpec`]) and the
//!   receive-side result ([`DecryptedOutput`]), plus the crypto-material newtypes ([`CommitmentBytes`],
//!   [`EncryptedDataBytes`], [`RangeProofBytes`], [`UtxoTagBytes`]).
//! - The entropy-injection bundle ([`StealthEntropy`] + [`PerOutputEntropy`]): **all** pinned randomness a stealth
//!   build consumes, in a **positional, documented** order so any host can reproduce the proofs bit-for-bit.

use serde::{Deserialize, Serialize};

use crate::{
    seed,
    types::{
        address::{ComponentAddressStr, ResourceAddressStr},
        bytes::{BuildSeed, PublicKeyBytes, SecretKeyBytes},
        error::OotleSdkError,
        numeric::BoundaryAmount,
    },
};

// These crypto-material newtypes use the same lowercase-hex wire contract as `crate::types::bytes`:
// a single lowercase-hex string (no `0x`), uppercase rejected on deserialize. The serde helpers and
// newtype macros below are local copies because the originals in `bytes.rs` are private to that
// module.

/// serde helper: a fixed-width byte array as lowercase hex.
mod fixed_hex {
    use serde::{Deserialize, Deserializer, Serializer, de::Error as _};

    pub fn serialize<S: Serializer, const N: usize>(bytes: &[u8; N], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&hex::encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>, const N: usize>(d: D) -> Result<[u8; N], D::Error> {
        let s = String::deserialize(d)?;
        if s.chars().any(|c| c.is_ascii_uppercase()) {
            return Err(D::Error::custom("expected lowercase hex"));
        }
        let v = hex::decode(&s).map_err(D::Error::custom)?;
        let arr: [u8; N] = v
            .try_into()
            .map_err(|v: Vec<u8>| D::Error::custom(format!("expected {N} bytes, got {}", v.len())))?;
        Ok(arr)
    }
}

/// serde helper: a variable-length byte vector as lowercase hex.
mod var_hex {
    use serde::{Deserialize, Deserializer, Serializer, de::Error as _};

    pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&hex::encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let s = String::deserialize(d)?;
        if s.chars().any(|c| c.is_ascii_uppercase()) {
            return Err(D::Error::custom("expected lowercase hex"));
        }
        hex::decode(&s).map_err(D::Error::custom)
    }
}

/// Generates a fixed-width byte newtype that serializes as lowercase hex. Local copy of
/// `bytes::fixed_byte_newtype!` (that macro is private to `bytes.rs`).
macro_rules! stealth_fixed_byte_newtype {
    ($(#[$meta:meta])* $name:ident, $len:expr) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub struct $name(#[serde(with = "fixed_hex")] pub [u8; $len]);

        impl $name {
            /// The fixed byte width.
            pub const LEN: usize = $len;

            /// Wraps a fixed-width byte array.
            pub const fn from_array(bytes: [u8; $len]) -> Self {
                Self(bytes)
            }

            /// Builds from a byte slice, erroring on a width mismatch.
            pub fn from_bytes(bytes: &[u8]) -> Result<Self, OotleSdkError> {
                let arr: [u8; $len] = bytes.try_into().map_err(|_| {
                    OotleSdkError::Validation(format!(
                        concat!(stringify!($name), ": expected {} bytes, got {}"),
                        $len,
                        bytes.len()
                    ))
                })?;
                Ok(Self(arr))
            }

            /// Parses from a lowercase-hex string.
            pub fn from_hex(s: &str) -> Result<Self, OotleSdkError> {
                let v = hex::decode(s).map_err(|e| {
                    OotleSdkError::Parse(format!(concat!(stringify!($name), ": invalid hex: {}"), e))
                })?;
                Self::from_bytes(&v)
            }

            /// Borrows the raw bytes.
            pub fn as_bytes(&self) -> &[u8] {
                &self.0
            }

            /// Returns the owned byte array.
            pub const fn into_array(self) -> [u8; $len] {
                self.0
            }

            /// Returns the lowercase-hex encoding.
            pub fn to_hex(&self) -> String {
                hex::encode(self.0)
            }
        }
    };
}

/// Generates a variable-length byte newtype that serializes as lowercase hex. Local copy of
/// `bytes::var_byte_newtype!` (that macro is private to `bytes.rs`).
macro_rules! stealth_var_byte_newtype {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub struct $name(#[serde(with = "var_hex")] pub Vec<u8>);

        impl $name {
            /// Wraps an owned byte vector.
            pub const fn from_vec(bytes: Vec<u8>) -> Self {
                Self(bytes)
            }

            /// Copies a byte slice into a new value.
            pub fn from_bytes(bytes: &[u8]) -> Self {
                Self(bytes.to_vec())
            }

            /// Parses from a lowercase-hex string.
            pub fn from_hex(s: &str) -> Result<Self, OotleSdkError> {
                let v = hex::decode(s).map_err(|e| {
                    OotleSdkError::Parse(format!(concat!(stringify!($name), ": invalid hex: {}"), e))
                })?;
                Ok(Self(v))
            }

            /// Borrows the raw bytes.
            pub fn as_bytes(&self) -> &[u8] {
                &self.0
            }

            /// Returns the owned byte vector.
            pub fn into_vec(self) -> Vec<u8> {
                self.0
            }

            /// Returns the lowercase-hex encoding.
            pub fn to_hex(&self) -> String {
                hex::encode(&self.0)
            }
        }
    };
}

stealth_fixed_byte_newtype!(
    /// A Pedersen commitment (32 bytes), boundary form. Public material — `Copy`/`Hash` are
    /// acceptable.
    CommitmentBytes,
    32
);

stealth_fixed_byte_newtype!(
    /// A 4-byte UTXO scanning tag, boundary form. Carries a `u32` stored as **little-endian** bytes;
    /// use [`UtxoTagBytes::from_u32`] / [`UtxoTagBytes::to_u32`] to cross to/from the integer form.
    UtxoTagBytes,
    4
);

stealth_var_byte_newtype!(
    /// A variable-length AEAD ciphertext (the encrypted output payload), boundary form.
    EncryptedDataBytes
);

stealth_var_byte_newtype!(
    /// A variable-length aggregated bulletproof, boundary form.
    RangeProofBytes
);

impl UtxoTagBytes {
    /// Builds a tag from its `u32` value, storing it **little-endian** (the representation these 4
    /// bytes round-trip through).
    pub const fn from_u32(value: u32) -> Self {
        Self(value.to_le_bytes())
    }

    /// Reads the tag back as a `u32` from its **little-endian** bytes.
    pub const fn to_u32(&self) -> u32 {
        u32::from_le_bytes(self.0)
    }
}

/// A boundary memo: the small, language-neutral subset of the internal stealth memo that the
/// confidential-transfer surface needs.
///
/// The internal memo carries more variants (U256, pay-ref, sender address); the boundary
/// deliberately exposes only the two general-purpose ones. The output path maps these to the
/// internal memo; the scan path maps a decoded internal memo back to one of these (other internal
/// variants surface as [`StealthMemo::Bytes`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StealthMemo {
    /// A UTF-8 text message.
    Message(String),
    /// Arbitrary bytes.
    Bytes(Vec<u8>),
}

/// The spend condition controlling how a stealth output's one-time key is derived.
///
/// The internal spend condition has a rich `AccessRule` variant; the boundary keeps only the two
/// cases the confidential-transfer surface exercises:
///
/// - [`StealthPayTo::StealthPublicKey`] — the default: the one-time key is derived from the recipient's stealth public
///   key.
/// - [`StealthPayTo::AccessRuleAllowAll`] — an `AccessRule::AllowAll` spend condition.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum StealthPayTo {
    /// The one-time key is derived from the recipient's stealth public key (the default).
    #[default]
    StealthPublicKey,
    /// The output is spendable under an `AccessRule::AllowAll`.
    AccessRuleAllowAll,
}

/// One stealth output to create.
///
/// Crypto material crosses as hex newtypes; the destination is the recipient's account + view
/// public keys (not an opaque `Address`), `amount`/`minimum_value_promise` are plain `u64`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StealthOutputSpec {
    /// Recipient account public key.
    pub destination_account_pk: PublicKeyBytes,
    /// Recipient view-only public key (used for AEAD key derivation + UTXO scanning tag).
    pub destination_view_pk: PublicKeyBytes,
    /// Blinded (confidential) output value in µTari — the value committed by the stealth UTXO.
    pub amount: u64,
    /// Revealed (plaintext) value in µTari deposited into [`destination_account_pk`]'s account,
    /// like a normal public transfer. Defaults to `0` (no revealed deposit) so existing JSON stays
    /// valid. The per-output revealed amounts must sum to
    /// [`StealthTransferIntent::revealed_output_amount`].
    ///
    /// [`destination_account_pk`]: StealthOutputSpec::destination_account_pk
    #[serde(default)]
    pub revealed_amount: u64,
    /// The resource being transferred.
    pub resource_address: ResourceAddressStr,
    /// Resource view key (drives the ElGamal viewable-balance proof). `None` if the resource has no
    /// view key.
    pub resource_view_key: Option<PublicKeyBytes>,
    /// Optional AEAD-encrypted memo embedded in the encrypted payload.
    pub memo: Option<StealthMemo>,
    /// Spend condition variant (how the one-time key is derived).
    pub pay_to: StealthPayTo,
    /// Optional UTXO scanning tag (4 bytes).
    pub utxo_tag: Option<UtxoTagBytes>,
    /// Minimum value the range proof commits to (must be `<= amount`).
    pub minimum_value_promise: u64,
}

/// One stealth UTXO the caller wants to spend.
///
/// Stealth inputs are on-chain UTXO substates: the mask is **not** on the wire — it is recovered by
/// fetching the substate and decrypting it with the owner account's spend secret. This record
/// carries only what the fetch-want + decrypt need.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StealthInputSpec {
    /// The on-chain Pedersen commitment identifying this UTXO.
    pub commitment: CommitmentBytes,
    /// Account whose spend secret can decrypt this UTXO (used in the fetch want and decryption).
    pub owner_account_pk: PublicKeyBytes,
}

/// The top-level confidential-transfer intent.
///
/// What the developer wants: send the stealth `outputs` of `resource_address` from `from_account`,
/// spending the stealth `inputs`, paying `fee`. Revealed in/out amounts model a public bucket
/// entering/leaving the confidential flow (zero when there is none).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StealthTransferIntent {
    /// Sender's account component.
    pub from_account: ComponentAddressStr,
    /// Resource being transferred.
    pub resource_address: ResourceAddressStr,
    /// Fee in µTari.
    pub fee: BoundaryAmount,
    /// Stealth inputs to spend (on-chain UTXOs).
    pub inputs: Vec<StealthInputSpec>,
    /// Stealth outputs to create.
    pub outputs: Vec<StealthOutputSpec>,
    /// Revealed amount entering as a bucket (µTari). Zero if no revealed input.
    pub revealed_input_amount: u64,
    /// Revealed amount leaving as a bucket (µTari). Zero if no revealed output.
    pub revealed_output_amount: u64,
    /// Optional earliest epoch.
    pub min_epoch: Option<u64>,
    /// The last epoch this transaction is valid in. Mandatory: every transaction has a bounded
    /// validity window, capped network-wide at `max_transaction_validity_epochs` past the current epoch.
    pub max_epoch: u64,
    /// Dry-run flag.
    pub dry_run: bool,
    /// Pay the fee from the account's revealed (XTR) vault even when there is no revealed input.
    ///
    /// The fee is always charged from `from_account` via `pay_fee`, which only the account key can
    /// authorize. That key seals automatically when `revealed_input_amount > 0`; a pure
    /// confidential-input spend has no revealed input, so without this flag the account never signs
    /// and the engine denies `pay_fee`. Setting it forces the account-key seal so the fee is
    /// authorized. The fee stays out of the balance proof (the confidential inputs/outputs balance on
    /// their own).
    #[serde(default)]
    pub pay_fee_from_revealed: bool,
}

impl StealthTransferIntent {
    /// Validates that the per-output revealed amounts reconcile with the top-level
    /// [`revealed_output_amount`](StealthTransferIntent::revealed_output_amount).
    ///
    /// `revealed_output_amount` is the load-bearing total the balance proof commits to; each output
    /// carries its own `revealed_amount` slice deposited into its destination account.
    ///
    /// When there is **at least one output**, the per-output slices must reconcile with the total:
    /// `sum(outputs[].revealed_amount) == revealed_output_amount`. The sum is computed with a checked
    /// add so an overflowing intent is rejected rather than wrapping.
    ///
    /// When there are **no outputs**, `revealed_output_amount` is an undeposited pass-through total
    /// (the revealed-only shape, where `revealed_input == revealed_output` and the engine handles
    /// the balance without a per-output deposit) — there are no slices to reconcile, so this is
    /// accepted as-is.
    ///
    /// Returns [`OotleSdkError::Validation`] on a mismatch (or overflow). Run before assembly.
    pub fn validate_revealed_outputs(&self) -> Result<(), OotleSdkError> {
        if self.outputs.is_empty() {
            return Ok(());
        }
        let mut sum: u64 = 0;
        for output in &self.outputs {
            sum = sum.checked_add(output.revealed_amount).ok_or_else(|| {
                OotleSdkError::Validation("sum of per-output revealed_amount overflows u64".to_string())
            })?;
        }
        if sum != self.revealed_output_amount {
            return Err(OotleSdkError::Validation(format!(
                "sum of per-output revealed_amount ({sum}) must equal revealed_output_amount ({})",
                self.revealed_output_amount
            )));
        }
        Ok(())
    }
}

/// An inbound stealth UTXO to scan — the receive-side input record.
///
/// This is the on-the-wire shape of a created stealth output, reduced to the fields the receiver
/// needs to decrypt + claim it. All crypto material crosses as hex byte newtypes; the spend
/// condition is the boundary [`StealthPayTo`] form, the same as the send-side
/// [`StealthOutputSpec::pay_to`]. `scan_stealth_output` consumes this.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InboundStealthOutput {
    /// The on-chain Pedersen commitment (32 bytes).
    pub commitment: CommitmentBytes,
    /// The AEAD-encrypted output payload (variable length).
    pub encrypted_data: EncryptedDataBytes,
    /// The sender's ephemeral public nonce `R` (32 bytes); the receiver pairs it with their view
    /// secret to re-derive the AEAD key (DH commutativity).
    pub sender_public_nonce: PublicKeyBytes,
    /// The spend authorisation controlling how the one-time key is derived (`StealthPublicKey` for a
    /// `Key` output addressed to a recipient account, `AccessRuleAllowAll` otherwise). For a
    /// `StealthPublicKey` output, the embedded one-time spend public key is [`spend_public_key`].
    ///
    /// [`spend_public_key`]: InboundStealthOutput::spend_public_key
    pub pay_to: StealthPayTo,
    /// For a `StealthPayTo::StealthPublicKey` output, the on-chain one-time spend public key
    /// (`SpendAuthorization::Key`). The scanner re-derives it from the view secret and the account
    /// public key to confirm the output is spendable by that account. `None` for `AccessRuleAllowAll`.
    pub spend_public_key: Option<PublicKeyBytes>,
    /// The UTXO scanning tag (4 bytes), when present. Verified against the tag the scanner derives from
    /// its view secret.
    pub utxo_tag: Option<UtxoTagBytes>,
    /// The resource being transferred (needed to re-derive the scanning tag).
    pub resource_address: ResourceAddressStr,
}

/// The receive-side result of scanning an inbound stealth UTXO.
///
/// `scan_stealth_output` produces this.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecryptedOutput {
    /// Recovered plaintext value (µTari).
    pub value: u64,
    /// Recovered Pedersen commitment mask (secret scalar). No `Copy`/`Hash`.
    pub mask: SecretKeyBytes,
    /// Decoded memo, if present and `skip_memo` was false.
    pub memo: Option<StealthMemo>,
    /// Whether the output is addressed to the scanning key. The AEAD decrypt failing yields `None`
    /// for the whole [`DecryptedOutput`], so this is always `true` when the struct is returned —
    /// kept for forward compatibility.
    pub is_mine: bool,
}

/// The per-output slice of [`StealthEntropy`].
///
/// **The field order is load-bearing**: each field maps to a specific crypto call in this exact
/// order; reading them in any other order produces different proofs. The documented wire order is:
///
/// 1. `mask` — the Pedersen commitment mask (output blinding factor).
/// 2. `sender_nonce` — the sender ephemeral nonce secret driving `public_nonce`.
/// 3. `aead_nonce` — the XChaCha20-Poly1305 AEAD nonce (see field docs for the 24/32 layout).
/// 4. `elgamal_nonce` — the ElGamal ephemeral nonce (only when a `resource_view_key` is set).
/// 5. `zk_nonces` — the three ZK nonces `[x_v, x_m, x_r]` (only when a `resource_view_key` is set).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PerOutputEntropy {
    /// Pedersen commitment mask (secret scalar).
    pub mask: SecretKeyBytes,
    /// Sender ephemeral nonce secret (drives `public_nonce`).
    pub sender_nonce: SecretKeyBytes,
    /// AEAD nonce. XChaCha20-Poly1305 uses a **24-byte** nonce, but this field is a 32-byte
    /// [`SecretKeyBytes`] so it stays uniform with every other entropy field: the **leading 24
    /// bytes** are the nonce and the **trailing 8 bytes are ignored**. A re-port must take exactly
    /// `aead_nonce[..24]`. This injected nonce is threaded into the seeded AEAD encrypt instead of
    /// an internally-drawn random nonce.
    pub aead_nonce: SecretKeyBytes,
    /// ElGamal ephemeral nonce secret (present only when `resource_view_key` is set).
    pub elgamal_nonce: Option<SecretKeyBytes>,
    /// Three ZK nonces `[x_v, x_m, x_r]` for the ElGamal viewable-balance proof (present only when
    /// `resource_view_key` is set).
    pub zk_nonces: Option<[SecretKeyBytes; 3]>,
}

/// The pinned per-output **construction** randomness a stealth build consumes.
///
/// Every field is a 32-byte secret; the bundle is **positional** — [`StealthEntropy::per_output`]
/// has exactly one [`PerOutputEntropy`] per output in
/// [`StealthTransferIntent::outputs`](crate::types::stealth::StealthTransferIntent::outputs), in
/// the same order. A length mismatch at build time is an
/// [`OotleSdkError::Validation`](crate::types::error::OotleSdkError::Validation).
///
/// The deterministic build entry points take `&StealthEntropy` and **never** call an RNG; the
/// production entry points fill one from the OS RNG via [`StealthEntropy::from_os_rng`] and then
/// call the deterministic path. That `from_os_rng` constructor is the **only** place RNG enters the
/// stealth *build* surface. Signatures (auth/seal) are signed later with fresh random nonces (see
/// [`crate::stealth::sign_seal`]) — no signing nonce is derived from a seed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StealthEntropy {
    /// One slice per output, in the same order as `StealthTransferIntent.outputs`.
    pub per_output: Vec<PerOutputEntropy>,
}

impl StealthEntropy {
    /// Builds a fully-pinned entropy bundle with **all** randomness drawn from the OS RNG, with one
    /// [`PerOutputEntropy`] per output (`num_outputs`).
    ///
    /// This is the **only** RNG entry point for the stealth surface; the `*_production` build paths
    /// call it, then run the deterministic path against the pinned result. Every per-output slice
    /// is filled with the optional ElGamal/ZK nonces present — the build ignores the unused slots
    /// when the corresponding output has no `resource_view_key`. Drawing them unconditionally keeps
    /// the bundle independent of the intent's view-key shape.
    ///
    /// Draws one [`BuildSeed`] from the OS RNG and expands it via [`StealthEntropy::from_seed`], so
    /// the random and seeded paths share one derivation — there is no separate "unsafe" branch.
    pub fn from_os_rng(num_outputs: usize) -> Self {
        Self::from_seed(&random_seed(), num_outputs)
    }

    /// Expands a single [`BuildSeed`] into a full entropy bundle with one [`PerOutputEntropy`] per
    /// output (`num_outputs`). Every scalar is derived by the domain-separated KDF in
    /// [`crate::seed`], so the same seed + the same `num_outputs` always reproduces identical bytes.
    ///
    /// Every per-output slice fills the optional ElGamal/ZK nonces; the build ignores the unused slots
    /// when the corresponding output has no `resource_view_key`. Filling them unconditionally keeps the
    /// bundle independent of the intent's view-key shape, matching the OS-RNG path exactly.
    ///
    /// The rails ([`StealthEntropy::validate`]) run before returning: a future KDF regression that
    /// produced a zero or duplicated scalar would surface here, not in shipped bytes.
    pub fn from_seed(seed: &BuildSeed, num_outputs: usize) -> Self {
        let per_output = (0..num_outputs)
            .map(|i| {
                let i = i as u64;
                PerOutputEntropy {
                    mask: seed::derive_mask(seed, i),
                    sender_nonce: seed::derive_sender_nonce(seed, i),
                    aead_nonce: seed::derive_aead_nonce(seed, i),
                    elgamal_nonce: Some(seed::derive_elgamal_nonce(seed, i)),
                    zk_nonces: Some(seed::derive_zk_nonces(seed, i)),
                }
            })
            .collect();
        let entropy = Self { per_output };
        entropy
            .validate()
            .expect("KDF-derived entropy is non-zero and pairwise-distinct by construction");
        entropy
    }

    /// Defense-in-depth rails over a derived bundle: every consumed secret must be non-zero and the
    /// flattened set pairwise-distinct. The KDF guarantees both; this assert catches a future KDF bug
    /// before degenerate bytes ship.
    ///
    /// The non-zero check on the AEAD nonce covers `[..24]` — the only slice XChaCha20-Poly1305
    /// consumes (the trailing 8 bytes are ignored, so an otherwise non-zero scalar with a zero
    /// 24-byte prefix is still a degenerate nonce). Pairwise distinctness compares the full 32-byte
    /// scalars uniformly: two identical AEAD nonces collide on all 32 bytes too, so this loses no
    /// detection power while keeping the comparison length-uniform.
    ///
    /// Returns [`OotleSdkError::Validation`] naming the offending field.
    pub fn validate(&self) -> Result<(), OotleSdkError> {
        // (label, full scalar bytes, consumed slice) for every secret in the bundle. The consumed
        // slice differs from the full bytes only for the AEAD nonce.
        let mut scalars: Vec<(String, &[u8], &[u8])> = Vec::new();
        for (i, po) in self.per_output.iter().enumerate() {
            scalars.push((format!("per_output[{i}].mask"), po.mask.as_bytes(), po.mask.as_bytes()));
            scalars.push((
                format!("per_output[{i}].sender_nonce"),
                po.sender_nonce.as_bytes(),
                po.sender_nonce.as_bytes(),
            ));
            scalars.push((
                format!("per_output[{i}].aead_nonce"),
                po.aead_nonce.as_bytes(),
                &po.aead_nonce.as_bytes()[..24],
            ));
            if let Some(n) = &po.elgamal_nonce {
                scalars.push((format!("per_output[{i}].elgamal_nonce"), n.as_bytes(), n.as_bytes()));
            }
            if let Some(zk) = &po.zk_nonces {
                for (k, n) in zk.iter().enumerate() {
                    scalars.push((format!("per_output[{i}].zk_nonces[{k}]"), n.as_bytes(), n.as_bytes()));
                }
            }
        }
        for (label, _, consumed) in &scalars {
            if consumed.iter().all(|&b| b == 0) {
                return Err(OotleSdkError::Validation(format!(
                    "derived entropy {label} is all-zero"
                )));
            }
        }
        for i in 0..scalars.len() {
            for j in (i + 1)..scalars.len() {
                if scalars[i].1 == scalars[j].1 {
                    return Err(OotleSdkError::Validation(format!(
                        "derived entropy {} collides with {}",
                        scalars[i].0, scalars[j].0
                    )));
                }
            }
        }
        Ok(())
    }
}

/// Draws a fresh 32-byte [`BuildSeed`] from the OS RNG. This is the single RNG entry point the random
/// build paths use; everything downstream is a deterministic expansion of this seed.
pub fn random_seed() -> BuildSeed {
    use rand::RngExt;
    BuildSeed::from_array(rand::rng().random::<[u8; 32]>())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn component_str() -> String {
        use tari_template_lib_types::{ComponentAddress, ObjectKey};
        ComponentAddress::new(ObjectKey::from_array([0xaa; ObjectKey::LENGTH])).to_string()
    }

    fn resource_str() -> String {
        use tari_template_lib_types::{ObjectKey, ResourceAddress};
        ResourceAddress::new(ObjectKey::from_array([0xbb; ObjectKey::LENGTH])).to_string()
    }

    fn sample_entropy() -> StealthEntropy {
        StealthEntropy {
            per_output: vec![PerOutputEntropy {
                mask: SecretKeyBytes::from_array([1; 32]),
                sender_nonce: SecretKeyBytes::from_array([2; 32]),
                aead_nonce: SecretKeyBytes::from_array([3; 32]),
                elgamal_nonce: Some(SecretKeyBytes::from_array([4; 32])),
                zk_nonces: Some([
                    SecretKeyBytes::from_array([5; 32]),
                    SecretKeyBytes::from_array([6; 32]),
                    SecretKeyBytes::from_array([7; 32]),
                ]),
            }],
        }
    }

    // (a) Serde round-trip — StealthEntropy.
    #[test]
    fn stealth_entropy_serde_round_trips() {
        let e = sample_entropy();
        let json = serde_json::to_string(&e).unwrap();
        let back: StealthEntropy = serde_json::from_str(&json).unwrap();
        assert_eq!(back, e);
        // Structural format check: snake_case field names present.
        for field in [
            "per_output",
            "mask",
            "sender_nonce",
            "aead_nonce",
            "elgamal_nonce",
            "zk_nonces",
        ] {
            assert!(json.contains(field), "missing field {field} in {json}");
        }
    }

    // (b) Serde round-trip — StealthTransferIntent (minimal).
    #[test]
    fn stealth_transfer_intent_serde_round_trips() {
        let intent = StealthTransferIntent {
            from_account: ComponentAddressStr::parse(component_str()).unwrap(),
            resource_address: ResourceAddressStr::parse(resource_str()).unwrap(),
            fee: BoundaryAmount::new(2000),
            inputs: vec![StealthInputSpec {
                commitment: CommitmentBytes::from_array([0x33; 32]),
                owner_account_pk: PublicKeyBytes::from_array([0x44; 32]),
            }],
            outputs: vec![StealthOutputSpec {
                destination_account_pk: PublicKeyBytes::from_array([0x55; 32]),
                destination_view_pk: PublicKeyBytes::from_array([0x66; 32]),
                amount: 5_000_000,
                revealed_amount: 0,
                resource_address: ResourceAddressStr::parse(resource_str()).unwrap(),
                resource_view_key: None,
                memo: None,
                pay_to: StealthPayTo::StealthPublicKey,
                utxo_tag: None,
                minimum_value_promise: 0,
            }],
            revealed_input_amount: 0,
            revealed_output_amount: 0,
            min_epoch: None,
            max_epoch: 99,
            dry_run: false,
            pay_fee_from_revealed: false,
        };
        let json = serde_json::to_string(&intent).unwrap();
        let back: StealthTransferIntent = serde_json::from_str(&json).unwrap();
        assert_eq!(back, intent);
    }

    // (c) Serde round-trip — StealthOutputSpec variants.
    #[test]
    fn stealth_output_spec_variants_round_trip() {
        let base = StealthOutputSpec {
            destination_account_pk: PublicKeyBytes::from_array([0x11; 32]),
            destination_view_pk: PublicKeyBytes::from_array([0x22; 32]),
            amount: 1_000_000,
            revealed_amount: 0,
            resource_address: ResourceAddressStr::parse(resource_str()).unwrap(),
            resource_view_key: None,
            memo: None,
            pay_to: StealthPayTo::StealthPublicKey,
            utxo_tag: None,
            minimum_value_promise: 0,
        };

        // (i) bare.
        let json = serde_json::to_string(&base).unwrap();
        assert_eq!(serde_json::from_str::<StealthOutputSpec>(&json).unwrap(), base);

        // (ii) with a Message memo.
        let with_memo = StealthOutputSpec {
            memo: Some(StealthMemo::Message("hello".into())),
            pay_to: StealthPayTo::AccessRuleAllowAll,
            ..base.clone()
        };
        let json = serde_json::to_string(&with_memo).unwrap();
        assert_eq!(serde_json::from_str::<StealthOutputSpec>(&json).unwrap(), with_memo);

        // (iii) with a resource view key and a utxo tag.
        let with_view = StealthOutputSpec {
            resource_view_key: Some(PublicKeyBytes::from_array([0x77; 32])),
            utxo_tag: Some(UtxoTagBytes::from_u32(0x0102_0304)),
            memo: Some(StealthMemo::Bytes(vec![1, 2, 3])),
            minimum_value_promise: 500_000,
            ..base.clone()
        };
        let json = serde_json::to_string(&with_view).unwrap();
        assert_eq!(serde_json::from_str::<StealthOutputSpec>(&json).unwrap(), with_view);

        // (iv) with a non-zero revealed_amount (the revealed-output deposit slice).
        let with_revealed = StealthOutputSpec {
            revealed_amount: 250_000,
            ..base
        };
        let json = serde_json::to_string(&with_revealed).unwrap();
        assert_eq!(serde_json::from_str::<StealthOutputSpec>(&json).unwrap(), with_revealed);

        // (v) `revealed_amount` defaults to 0 when omitted (forward-compat with older JSON).
        let without_field = serde_json::json!({
            "destination_account_pk": "11".repeat(32),
            "destination_view_pk": "22".repeat(32),
            "amount": 1_000_000,
            "resource_address": resource_str(),
            "resource_view_key": null,
            "memo": null,
            "pay_to": "StealthPublicKey",
            "utxo_tag": null,
            "minimum_value_promise": 0,
        });
        assert_eq!(
            serde_json::from_value::<StealthOutputSpec>(without_field)
                .unwrap()
                .revealed_amount,
            0,
            "omitted revealed_amount must default to 0"
        );
    }

    // (c2) The intent's revealed-output sum validation.
    #[test]
    fn revealed_output_sum_validation() {
        fn output(amount: u64, revealed: u64) -> StealthOutputSpec {
            StealthOutputSpec {
                destination_account_pk: PublicKeyBytes::from_array([0x11; 32]),
                destination_view_pk: PublicKeyBytes::from_array([0x22; 32]),
                amount,
                revealed_amount: revealed,
                resource_address: ResourceAddressStr::parse(resource_str()).unwrap(),
                resource_view_key: None,
                memo: None,
                pay_to: StealthPayTo::StealthPublicKey,
                utxo_tag: None,
                minimum_value_promise: 0,
            }
        }
        fn intent(outputs: Vec<StealthOutputSpec>, revealed_output_amount: u64) -> StealthTransferIntent {
            StealthTransferIntent {
                from_account: ComponentAddressStr::parse(component_str()).unwrap(),
                resource_address: ResourceAddressStr::parse(resource_str()).unwrap(),
                fee: BoundaryAmount::new(2000),
                inputs: vec![],
                outputs,
                revealed_input_amount: 0,
                revealed_output_amount,
                min_epoch: None,
                max_epoch: 1,
                dry_run: false,
                pay_fee_from_revealed: false,
            }
        }

        // Matching sum (multi-output split) ⇒ Ok.
        intent(vec![output(1, 300_000), output(1, 200_000)], 500_000)
            .validate_revealed_outputs()
            .unwrap();
        // No revealed outputs, total 0 ⇒ Ok.
        intent(vec![output(1, 0)], 0).validate_revealed_outputs().unwrap();
        // Mismatch ⇒ Validation error.
        let err = intent(vec![output(1, 300_000), output(1, 100_000)], 500_000)
            .validate_revealed_outputs()
            .unwrap_err();
        assert!(matches!(err, OotleSdkError::Validation(_)));
    }

    // (d) Serde round-trip — DecryptedOutput.
    #[test]
    fn decrypted_output_serde_round_trips() {
        let d = DecryptedOutput {
            value: 1_234_567,
            mask: SecretKeyBytes::from_array([0xcd; 32]),
            memo: Some(StealthMemo::Message("memo".into())),
            is_mine: true,
        };
        let json = serde_json::to_string(&d).unwrap();
        assert_eq!(serde_json::from_str::<DecryptedOutput>(&json).unwrap(), d);
    }

    // (e) Hex enforced lowercase.
    #[test]
    fn commitment_hex_is_lowercase_and_rejects_uppercase() {
        let json = serde_json::to_string(&CommitmentBytes::from_array([0xAB; 32])).unwrap();
        assert_eq!(json, format!("\"{}\"", "ab".repeat(32)));
        let err = serde_json::from_str::<CommitmentBytes>(&format!("\"{}\"", "AB".repeat(32)));
        assert!(err.is_err());
    }

    // (f) UtxoTagBytes round-trips u32.
    #[test]
    fn utxo_tag_round_trips_u32() {
        let tag = UtxoTagBytes::from_u32(0xDEAD_BEEF);
        assert_eq!(tag.to_u32(), 0xDEAD_BEEF);
        let json = serde_json::to_string(&tag).unwrap();
        assert_eq!(tag, serde_json::from_str::<UtxoTagBytes>(&json).unwrap());
    }

    // (g) Secret newtypes omit Copy/Hash — the entropy secret fields are SecretKeyBytes (the
    // macro that omits Copy/Hash), not PublicKeyBytes. A move-after-use proves no `Copy`.
    #[test]
    fn entropy_secret_fields_are_not_copy() {
        let e = sample_entropy();
        let moved = e.per_output[0].mask.clone();
        let moved_again = moved;
        assert_eq!(moved_again, SecretKeyBytes::from_array([1; 32]));
        // `moved` is moved-from here — it would not compile if `SecretKeyBytes` were `Copy`, which is
        // the secrecy guarantee we want.
    }

    // (h) from_os_rng is random across two calls.
    #[test]
    fn from_os_rng_is_random() {
        let a = StealthEntropy::from_os_rng(2);
        let b = StealthEntropy::from_os_rng(2);
        assert_eq!(a.per_output.len(), 2);
        assert_ne!(a, b);
    }

    #[test]
    fn from_os_rng_zero_outputs() {
        let e = StealthEntropy::from_os_rng(0);
        assert!(e.per_output.is_empty());
    }

    // (h2) from_os_rng still yields a rails-passing bundle on every draw.
    #[test]
    fn from_os_rng_passes_rails() {
        for _ in 0..8 {
            StealthEntropy::from_os_rng(3).validate().unwrap();
        }
    }

    // (i) from_seed is deterministic: same seed + count ⇒ identical bundle.
    #[test]
    fn from_seed_is_deterministic() {
        let seed = BuildSeed::from_array([0x11; 32]);
        assert_eq!(StealthEntropy::from_seed(&seed, 2), StealthEntropy::from_seed(&seed, 2));
    }

    // (j) Distinct seeds yield distinct bundles.
    #[test]
    fn from_seed_distinct_seeds_differ() {
        let a = StealthEntropy::from_seed(&BuildSeed::from_array([0x11; 32]), 2);
        let b = StealthEntropy::from_seed(&BuildSeed::from_array([0x22; 32]), 2);
        assert_ne!(a, b);
    }

    // (k) Index binding (D3): the same per-output field differs across outputs, so cross-output
    // reuse of x_m (the leak vector) is structurally impossible.
    #[test]
    fn from_seed_binds_output_index() {
        let e = StealthEntropy::from_seed(&BuildSeed::from_array([0x11; 32]), 2);
        let xm0 = &e.per_output[0].zk_nonces.as_ref().unwrap()[1];
        let xm1 = &e.per_output[1].zk_nonces.as_ref().unwrap()[1];
        assert_ne!(xm0, xm1, "x_m must differ across outputs");
        assert_ne!(e.per_output[0].mask, e.per_output[1].mask);
    }

    // (l) A freshly-derived bundle passes the rails (non-zero + pairwise-distinct).
    #[test]
    fn from_seed_passes_rails() {
        StealthEntropy::from_seed(&BuildSeed::from_array([0x11; 32]), 3)
            .validate()
            .unwrap();
    }

    // (m) The rails reject a hand-built bundle with a duplicated scalar.
    #[test]
    fn validate_rejects_duplicate_scalar() {
        let mut e = sample_entropy();
        e.per_output[0].sender_nonce = e.per_output[0].mask.clone();
        let err = e.validate().unwrap_err();
        assert!(matches!(err, OotleSdkError::Validation(_)));
    }

    // (n) The rails reject an all-zero consumed scalar.
    #[test]
    fn validate_rejects_zero_scalar() {
        let mut e = sample_entropy();
        e.per_output[0].mask = SecretKeyBytes::from_array([0; 32]);
        let err = e.validate().unwrap_err();
        assert!(matches!(err, OotleSdkError::Validation(_)));
    }

    // (n2) The AEAD nonce rail checks the consumed [..24] slice: a zero 24-byte prefix is rejected
    // even when the ignored trailing 8 bytes are non-zero.
    #[test]
    fn validate_rejects_zero_aead_prefix() {
        let mut e = sample_entropy();
        let mut bytes = [0u8; 32];
        bytes[24..].copy_from_slice(&[0xff; 8]);
        e.per_output[0].aead_nonce = SecretKeyBytes::from_array(bytes);
        let err = e.validate().unwrap_err();
        assert!(matches!(err, OotleSdkError::Validation(_)));
    }

    // (o) The seed rail rejects the all-zero seed but accepts a non-zero one.
    #[test]
    fn build_seed_rejects_zero() {
        assert!(matches!(
            BuildSeed::from_array([0; 32]).validate_nonzero().unwrap_err(),
            OotleSdkError::Validation(_)
        ));
        BuildSeed::from_array([0x01; 32]).validate_nonzero().unwrap();
    }

    // (p) random_seed draws differ across calls and are non-zero.
    #[test]
    fn random_seed_is_random_nonzero() {
        let a = random_seed();
        let b = random_seed();
        assert_ne!(a, b);
        a.validate_nonzero().unwrap();
    }

    // Crypto-material newtype basics still round-trip via the macro impls.
    #[test]
    fn var_newtypes_round_trip() {
        let ed = EncryptedDataBytes::from_bytes(&[9, 8, 7]);
        assert_eq!(EncryptedDataBytes::from_hex(&ed.to_hex()).unwrap(), ed);
        let rp = RangeProofBytes::from_bytes(&[1, 2]);
        assert_eq!(RangeProofBytes::from_hex(&rp.to_hex()).unwrap(), rp);
    }

    #[test]
    fn commitment_from_bytes_rejects_wrong_width() {
        let err = CommitmentBytes::from_bytes(&[0u8; 31]).unwrap_err();
        assert!(matches!(err, OotleSdkError::Validation(_)));
    }
}
