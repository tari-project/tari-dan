//    Copyright 2026 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use minicbor::{CborLen, Decode, Decoder, Encode, decode};
use tari_bor::adapters::bounded_vec;
use tari_template_abi::rust::prelude::*;

use super::SpendCondition;
use crate::{Hash32, bytes::Bytes};

/// Selects, per spent stealth input, which authorisation path the spender is exercising (TIP-0006).
///
/// The spend is witness-driven: the committed output's [`SpendAuthorization`](super::SpendAuthorization) only gates
/// which witnesses are *admissible* (its `spend_key` for the key path, its `condition_root` for the script path); the
/// engine never assumes which path the spender picks. This type is pure data — no hashing — so it is safe to compile
/// into a template; the engine recomputes the Merkle root natively.
#[derive(Debug, Clone, Default, Encode, Decode, CborLen, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[cfg_attr(feature = "borsh", derive(borsh::BorshSerialize))]
pub enum SpendWitness {
    /// Key-path spend: authorised by a signature from the output's `spend_key`, proven via the transaction's auth
    /// scope (a signer badge). Carries no payload — the signature lives in the transaction envelope.
    #[n(0)]
    #[default]
    KeyPath,
    /// Script-path spend: reveals one committed [`SpendCondition`] leaf and its inclusion proof against the output's
    /// `condition_root`. The engine recomputes the root, and on a match evaluates the leaf.
    ///
    /// `data` is a single spender-supplied witness blob the revealed leaf may interpret — a hashlock preimage, a
    /// signature, or a CBOR-encoded structure a `TemplateFunction` decodes. It is **not** committed in
    /// `condition_root` (only the leaf is), so it cannot alter which predicate runs, only satisfy one. A
    /// data-consuming builtin (e.g. a hashlock) reads it as raw bytes and must be the sole consumer in its leaf; a
    /// `TemplateFunction` reads it via `SpendContext::data`. Bounded by `STEALTH_LIMITS.max_witness_data_len`.
    #[n(1)]
    ScriptPath {
        #[n(0)]
        leaf: SpendCondition,
        #[n(1)]
        proof: MerkleProof,
        #[n(2)]
        data: Bytes,
    },
    // `#[n(2)]` is reserved for a future `ScriptPathTweaked` variant (the deferred taproot-style key tweak); do not
    // reuse this index.
}

impl SpendWitness {
    /// A script-path spend with no witness data (the common case for access-rule, covenant and timelock leaves).
    pub fn script_path(leaf: SpendCondition, proof: MerkleProof) -> Self {
        Self::ScriptPath {
            leaf,
            proof,
            data: Bytes::default(),
        }
    }

    /// A script-path spend supplying a witness `data` blob the revealed leaf interprets (e.g. a hashlock preimage).
    pub fn script_path_with_data(leaf: SpendCondition, proof: MerkleProof, data: Bytes) -> Self {
        Self::ScriptPath { leaf, proof, data }
    }

    pub const fn is_key_path(&self) -> bool {
        matches!(self, Self::KeyPath)
    }

    pub fn as_script_path(&self) -> Option<(&SpendCondition, &MerkleProof, &Bytes)> {
        match self {
            Self::ScriptPath { leaf, proof, data } => Some((leaf, proof, data)),
            _ => None,
        }
    }
}

/// An inclusion proof for a condition-tree (MAST) leaf: the sibling hashes from the leaf up to the root, bottom-first.
///
/// Carries no direction bits — the engine re-sorts each pair lexicographically on the way up. This type is pure data
/// (no hashing), so it lives beside the spend witness in the WASM-safe crate while the hashing stays native in
/// `tari_engine_types`.
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode, CborLen, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[cfg_attr(feature = "borsh", derive(borsh::BorshSerialize))]
pub struct MerkleProof {
    #[n(0)]
    #[cbor(decode_with = "decode_siblings")]
    pub siblings: Vec<Hash32>,
}

/// The most sibling hashes one [`MerkleProof`] may hold. Decoding refuses a longer proof.
pub const MAX_INCLUSION_PROOF_LEN: usize = 32;

fn decode_siblings<'b, C>(d: &mut Decoder<'b>, ctx: &mut C) -> Result<Vec<Hash32>, decode::Error> {
    bounded_vec::decode(d, ctx, MAX_INCLUSION_PROOF_LEN)
}

impl MerkleProof {
    pub fn new(siblings: Vec<Hash32>) -> Self {
        Self { siblings }
    }

    /// A proof for a single-leaf tree, where the leaf is itself the root.
    pub fn empty() -> Self {
        Self { siblings: Vec::new() }
    }
}
