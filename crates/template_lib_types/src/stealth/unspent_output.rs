//    Copyright 2025 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use minicbor::{CborLen, Decode, Encode};
use tari_bor::adapters::boxed_slice;
use tari_template_abi::rust::prelude::*;

use super::ViewableBalanceProof;
use crate::{
    EncryptedData,
    FunctionName,
    Hash32,
    TemplateAddress,
    access_rules::AccessRule,
    bytes::Bytes,
    crypto::{PedersenCommitmentBytes, RistrettoPublicKeyBytes, UtxoTag},
};

/// An unspent output that does not reveal the value and the owner of the coin it represents.
///
/// Unspent outputs contain:
/// - **commitment** - the Pedersen commitment k.G + v.H
/// - **sender_public_nonce** - the sender-provided public nonce that is used as part of a DH key exchange to generate
///   the decryption key for the encrypted data.
/// - **encrypted_data** - the encrypted data that contains the encrypted mask and value.
/// - **viewable_balance_proof** - an optional verifiable balance proof that must be provided and valid if the view key
///   is enabled for a resource.
#[derive(Debug, Clone, Encode, Decode, CborLen, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[cfg_attr(feature = "borsh", derive(borsh::BorshSerialize))]
pub struct UnspentOutput {
    #[n(0)]
    pub commitment: PedersenCommitmentBytes,
    /// Public nonce (R) that was used to generate the commitment mask
    #[n(1)]
    pub sender_public_nonce: RistrettoPublicKeyBytes,
    /// Encrypted mask and value for the recipient.
    #[n(2)]
    pub encrypted_data: EncryptedData,
    #[n(3)]
    #[cfg_attr(feature = "serde", serde(deserialize_with = "ootle_serde::str_number::deserialize"))]
    #[cfg_attr(feature = "ts", ts(type = "number | bigint | string"))]
    pub minimum_value_promise: u64,
    /// If the view key is enabled for a given resource, this proof MUST be provided, otherwise it MUST NOT.
    #[n(4)]
    pub viewable_balance_proof: Option<ViewableBalanceProof>,
}

/// How a stealth output is authorised at spend time (TIP-0006): a key path, a committed condition tree, or both.
///
/// Modelled as an enum rather than a pair of `Option`s so the unspendable `{no key, no conditions}` state is
/// unrepresentable — by construction in memory and at the decode boundary, with no runtime invariant to enforce.
///
/// - **Key** — a one-time "stealth" public key; spendable by proving ownership of it (a signature, via the
///   transaction's auth scope).
/// - **Script** — the Merkle root (MAST) committing the set of alternative [`SpendCondition`] leaves; spendable by
///   revealing one leaf plus an inclusion proof.
/// - **KeyAndScript** — either path is admissible; the per-input `SpendWitness` selects which.
#[derive(Debug, Clone, Encode, Decode, CborLen, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[cfg_attr(feature = "borsh", derive(borsh::BorshSerialize))]
pub enum SpendAuthorization {
    #[n(0)]
    Key(#[n(0)] RistrettoPublicKeyBytes),
    #[n(1)]
    Script(#[n(0)] Hash32),
    #[n(2)]
    KeyAndScript {
        #[n(0)]
        spend_key: RistrettoPublicKeyBytes,
        #[n(1)]
        condition_root: Hash32,
    },
}

impl SpendAuthorization {
    /// The one-time key authorising a key-path spend, if this output has a key path.
    pub fn spend_key(&self) -> Option<&RistrettoPublicKeyBytes> {
        match self {
            Self::Key(key) | Self::KeyAndScript { spend_key: key, .. } => Some(key),
            Self::Script(_) => None,
        }
    }

    /// The committed condition-tree root authorising a script-path spend, if this output has a condition tree.
    pub fn condition_root(&self) -> Option<&Hash32> {
        match self {
            Self::Script(root) |
            Self::KeyAndScript {
                condition_root: root, ..
            } => Some(root),
            Self::Key(_) => None,
        }
    }
}

/// A stealth unspent output, authorised at spend time per its [`SpendAuthorization`] (TIP-0006).
#[derive(Debug, Clone, Encode, Decode, CborLen, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[cfg_attr(feature = "borsh", derive(borsh::BorshSerialize))]
pub struct StealthUnspentOutput {
    #[n(0)]
    pub output: UnspentOutput,
    #[n(1)]
    pub auth: SpendAuthorization,
    #[n(2)]
    pub tag: UtxoTag,
}

impl StealthUnspentOutput {
    pub fn commitment(&self) -> &PedersenCommitmentBytes {
        &self.output.commitment
    }
}

/// A spend-condition leaf (v0) committed in a stealth output's [`SpendAuthorization`] condition tree: a flat, non-empty
/// conjunction (logical AND) of [`AtomicCondition`]s. A script-path spend reveals one leaf and an inclusion proof; the
/// engine recomputes the root and, on a match, requires every atom in the conjunction to hold.
///
/// Disjunction (OR) is the condition tree itself — each committed leaf is an alternative spend path — so a leaf needs
/// no OR combinator, only this AND. Atoms are not self-referential, so a leaf cannot nest another leaf: there is no
/// recursion to bound at decode and no nesting to reject at spend time.
#[derive(Debug, Clone, Encode, Decode, CborLen, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize), serde(transparent))]
#[cfg_attr(feature = "borsh", derive(borsh::BorshSerialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct SpendCondition(
    #[n(0)]
    #[cbor(with = "boxed_slice")]
    Box<[AtomicCondition]>,
);

impl SpendCondition {
    /// A conjunction leaf requiring every given atom to hold. Must be non-empty and within
    /// `STEALTH_LIMITS.max_conditions_per_conjunction`, both checked at spend time.
    pub fn all(conditions: impl IntoIterator<Item = AtomicCondition>) -> Self {
        Self(conditions.into_iter().collect())
    }

    /// A single-atom conjunction leaf.
    pub fn single(condition: AtomicCondition) -> Self {
        Self(Box::new([condition]))
    }

    /// A single-atom leaf gated on a native access rule.
    pub fn access_rule(rule: AccessRule) -> Self {
        Self::single(AtomicCondition::AccessRule(rule))
    }

    /// A single-atom leaf gated on a WASM predicate.
    pub fn template_function(func: TemplateFunction) -> Self {
        Self::single(AtomicCondition::TemplateFunction(func))
    }

    /// A single-atom leaf gated on a native [`BuiltinPredicate`].
    pub fn builtin(predicate: BuiltinPredicate) -> Self {
        Self::single(AtomicCondition::Builtin(predicate))
    }

    /// A single-atom leaf gated on a native [`Covenant`].
    pub fn covenant(covenant: Covenant) -> Self {
        Self::single(AtomicCondition::Covenant(covenant))
    }

    /// The conjoined atoms, all of which must hold for the leaf to admit the spend.
    pub fn conditions(&self) -> &[AtomicCondition] {
        &self.0
    }

    /// Consumes the leaf, yielding its conjoined atoms.
    pub fn into_conditions(self) -> Box<[AtomicCondition]> {
        self.0
    }
}

/// One conjunct of a [`SpendCondition`] leaf. Atoms are not self-referential — composition lives in the enclosing
/// conjunction (AND) and the condition tree (OR), never inside an atom — so the type carries no recursion to bound.
#[derive(Debug, Clone, Encode, Decode, CborLen, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "borsh", derive(borsh::BorshSerialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum AtomicCondition {
    /// Gated on a native access rule evaluated against the auth scope of the call frame performing the spend, as
    /// for a component method or resource rule: caller badges name that frame's immediate caller, and proofs held
    /// in that frame count towards the rule.
    #[n(0)]
    AccessRule(#[n(0)] AccessRule),
    /// Gated on a stateless WASM predicate over the spending transfer. The referenced template function introspects
    /// the spending `StealthTransferStatement` and rejects the spend by panicking.
    #[n(1)]
    TemplateFunction(#[n(0)] TemplateFunction),
    /// Gated on a native [`BuiltinPredicate`] — a consensus-fixed local spend predicate (timelock or hashlock) that
    /// gates this input on a local fact, needs no deployed template, and is evaluated natively.
    #[n(2)]
    Builtin(#[n(0)] BuiltinPredicate),
    /// Gated on a native [`Covenant`] — a consensus-fixed constraint over the spending transaction's outputs and value
    /// flow. It constrains the resulting transaction rather than gating this input on a local fact.
    #[n(3)]
    Covenant(#[n(0)] Covenant),
}

impl AtomicCondition {
    pub const fn is_template_function(&self) -> bool {
        matches!(self, Self::TemplateFunction(_))
    }

    /// Whether this atom is a builtin that reads the witness `data` blob as its own complete input. Such a builtin owns
    /// the whole blob, so it must be the sole `data` consumer in its leaf (enforced at spend time).
    pub const fn is_data_owning_builtin(&self) -> bool {
        matches!(self, Self::Builtin(predicate) if predicate.consumes_data())
    }
}

/// A stateless WASM predicate that gates the spend of a stealth UTXO.
///
/// The condition fully commits to `{template, function, args}`, so a spender cannot substitute a different predicate.
/// Templates are immutable substates, so the referenced code cannot change once the output is committed.
#[derive(Debug, Clone, Encode, Decode, CborLen, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[cfg_attr(feature = "borsh", derive(borsh::BorshSerialize))]
pub struct TemplateFunction {
    /// The template providing the predicate.
    #[n(0)]
    pub template: TemplateAddress,
    /// The stateless (`is_mut == false`) predicate function on that template.
    #[n(1)]
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub function: FunctionName,
    /// Bound parameters, positional — one CBOR-encoded value per leading (non-`SpendContext`) parameter, matching the
    /// engine's `Vec<Bytes>` call ABI. Baked into the condition at creation; the spender cannot alter them.
    #[n(2)]
    pub args: Vec<Bytes>,
}

impl TemplateFunction {
    pub fn new(template: TemplateAddress, function: FunctionName, args: Vec<Bytes>) -> Self {
        Self {
            template,
            function,
            args,
        }
    }
}

/// A native, consensus-fixed local spend predicate committed as an [`AtomicCondition::Builtin`] leaf (TIP-0006): a
/// timelock or hashlock that gates *this input* on a local fact. Constraints on the spending transaction's outputs are
/// a [`Covenant`] instead.
///
/// Unlike a [`TemplateFunction`], a builtin requires no deployed template and is evaluated natively by the engine —
/// the canonical semantics live in trusted core code, so the set is append-only and a shipped variant is never
/// resemanticised. Each variant is a pure boolean predicate over the spending transfer; rejecting a spend leaves its
/// inputs unspent. A predicate that needs spender-supplied data reads the witness `data` blob
/// ([`SpendWitness::ScriptPath`](super::SpendWitness::ScriptPath)) as its complete raw input — see
/// [`BuiltinPredicate::consumes_data`] and the sole-consumer rule it implies.
#[derive(Debug, Clone, Encode, Decode, CborLen, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[cfg_attr(feature = "borsh", derive(borsh::BorshSerialize))]
pub enum BuiltinPredicate {
    /// Absolute epoch lock: admissible only once the current epoch is at or after `unlock_epoch`.
    #[n(0)]
    AfterEpoch(
        #[n(0)]
        #[cfg_attr(feature = "serde", serde(deserialize_with = "ootle_serde::str_number::deserialize"))]
        #[cfg_attr(feature = "ts", ts(type = "number | bigint | string"))]
        u64,
    ),
    /// Absolute epoch deadline: admissible only while the current epoch is strictly before `deadline_epoch`.
    #[n(1)]
    BeforeEpoch(
        #[n(0)]
        #[cfg_attr(feature = "serde", serde(deserialize_with = "ootle_serde::str_number::deserialize"))]
        #[cfg_attr(feature = "ts", ts(type = "number | bigint | string"))]
        u64,
    ),
    /// Hashlock: the witness `data` blob must be a preimage whose `alg` digest equals `hash`. As a data-consuming
    /// builtin it reads the entire blob as raw bytes, so it must be the sole `data` consumer in its leaf.
    ///
    /// A bare hashlock is satisfiable by anyone who learns the preimage; pair it with an [`AccessRule`] in the same
    /// [`SpendCondition`] conjunction to bind the claim to a key (the standard HTLC construction).
    #[n(2)]
    HashLock {
        #[n(0)]
        hash: Hash32,
        #[n(1)]
        alg: HashAlg,
    },
}

impl BuiltinPredicate {
    /// Whether this predicate reads the witness `data` blob (as raw bytes) as its complete input. Such a predicate
    /// cannot know the blob's shape relative to siblings, so it owns the whole blob and must be the sole `data`
    /// consumer in its leaf.
    pub const fn consumes_data(&self) -> bool {
        match self {
            Self::HashLock { .. } => true,
            Self::AfterEpoch(_) | Self::BeforeEpoch(_) => false,
        }
    }
}

/// A native, consensus-fixed covenant committed as an [`AtomicCondition::Covenant`] leaf (TIP-0006). Where a
/// [`BuiltinPredicate`] gates an input on a local fact, a covenant constrains the *spending transaction* — which
/// outputs must exist, or that the partition's value is conserved — so it propagates conditions forward. Each
/// variant introspects the transfer's outputs natively, with canonical semantics in trusted core code. The set is a
/// curated standard library of common value-routing constraints; anything bespoke is a [`TemplateFunction`] instead.
#[derive(Debug, Clone, Encode, Decode, CborLen, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[cfg_attr(feature = "borsh", derive(borsh::BorshSerialize))]
pub enum Covenant {
    /// "Stay in the vault": every stealth output of the transfer must preserve the invoking `condition_root`, and there
    /// must be at least one such output.
    #[n(0)]
    OutputPreservesCondition,
    /// Value-flow: at least one stealth output must commit `condition_root` and promise at least `min_value`.
    #[n(1)]
    OutputTo {
        #[n(0)]
        condition_root: Hash32,
        #[n(1)]
        #[cfg_attr(feature = "serde", serde(deserialize_with = "ootle_serde::str_number::deserialize"))]
        #[cfg_attr(feature = "ts", ts(type = "number | bigint | string"))]
        min_value: u64,
    },
    /// Value-conservation (TIP-0006 Option A/C): the invoking partition's committed value is conserved into outputs
    /// carrying its `condition_root`, save for an exact cleartext outflow of at most `max_revealed`. A `max_revealed`
    /// of zero admits no cleartext escape (full conservation).
    #[n(2)]
    BalancePreserved(
        #[n(0)]
        #[cfg_attr(feature = "serde", serde(deserialize_with = "ootle_serde::str_number::deserialize"))]
        #[cfg_attr(feature = "ts", ts(type = "number | bigint | string"))]
        u64,
    ),
}

/// The digest used by a [`BuiltinPredicate::HashLock`]. The preimage is hashed with no domain separation so a hashlock
/// can interoperate with an external chain's HTLC (e.g. Bitcoin's `SHA256`).
#[derive(Debug, Clone, Copy, Encode, Decode, CborLen, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[cfg_attr(feature = "borsh", derive(borsh::BorshSerialize))]
pub enum HashAlg {
    #[n(0)]
    Blake2b256,
    #[n(1)]
    Sha256,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spend_condition_roundtrips() {
        let cases = [
            SpendCondition::access_rule(AccessRule::AllowAll),
            SpendCondition::template_function(TemplateFunction::new(
                Hash32::from_array([0u8; 32]),
                FunctionName::try_from("transfer").unwrap(),
                vec![Bytes::from_vec(vec![1, 2, 3])],
            )),
            SpendCondition::builtin(BuiltinPredicate::AfterEpoch(7)),
            SpendCondition::covenant(Covenant::BalancePreserved(0)),
            SpendCondition::all([
                AtomicCondition::Builtin(BuiltinPredicate::AfterEpoch(0)),
                AtomicCondition::AccessRule(AccessRule::AllowAll),
            ]),
        ];
        for cond in cases {
            let bytes = tari_bor::encode(&cond).unwrap();
            let decoded: SpendCondition = tari_bor::decode(&bytes).unwrap();
            assert_eq!(decoded, cond);
        }
    }
}
