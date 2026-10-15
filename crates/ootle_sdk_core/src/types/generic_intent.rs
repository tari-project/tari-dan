//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Generic transaction intent boundary + a typed argument DSL.
//!
//! The front-end that generalizes the hardcoded intents (public transfer / stealth) into an explicit
//! instruction list plus a typed argument DSL the core encodes to engine wire bytes. [`encode_arg`]
//! maps every [`ArgValue`] onto the builder's [`InstructionArg`] encoder, so argument encoding is
//! shared rather than reimplemented per host language.
//!
//! Types + encoding only. Two builder-stateful pieces are resolved during builder composition, not
//! here:
//!
//! - [`InstructionSpec::PutLastInstructionOutputOnWorkspace`] carries a label string; the numeric workspace id is
//!   assigned by the builder.
//! - [`ArgValue::Workspace`] names a workspace reference; its numeric [`WorkspaceOffsetId`] is resolved at the call
//!   point, never pre-resolved here.
//!
//! The boundary leaks no `tari_*` types: instructions reference addresses / keys by the boundary
//! newtypes ([`PublicKeyBytes`]) and plain strings (plain serde derive, no `rename_all`).
//!
//! ## Two instruction phases
//!
//! The engine runs **fee instructions first**, drops the workspace, then runs the **main
//! instructions**. So a self-funding transaction (create an account, fund it, pay the fee from it)
//! must place its create/fund/pay sequence in the fee phase. [`GenericTransactionIntent`] exposes
//! both phases: [`GenericTransactionIntent::fee_instructions`] (run first) and
//! [`GenericTransactionIntent::instructions`] (run after). [`FeeSource`] picks where the fee is
//! charged from.

use std::{collections::BTreeMap, str::FromStr};

use serde::{Deserialize, Serialize};
use tari_engine_types::substate::SubstateId;
use tari_ootle_transaction::args::{InstructionArg, WorkspaceOffsetId};
use tari_template_lib_types::{Amount, Metadata};

use crate::types::{
    address::ComponentAddressStr,
    bytes::PublicKeyBytes,
    error::OotleSdkError,
    intent::InputRef,
    numeric::BoundaryAmount,
};

/// A transaction blob (e.g. the WASM binary for `publish_template`), referenced by index from a
/// [`InstructionSpec::PublishTemplate`]. The bytes cross the boundary as lowercase hex.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobSpec {
    /// The raw blob bytes, lowercase hex (no `0x`).
    #[serde(serialize_with = "blob_hex::serialize", deserialize_with = "blob_hex::deserialize")]
    pub bytes: Vec<u8>,
}

/// serde helper: a blob's bytes as lowercase hex (rejects uppercase, matching the byte newtypes).
mod blob_hex {
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

/// serde helper: an optional lowercase-hex string (rejects uppercase at the boundary, matching
/// [`blob_hex`]). Kept as a `String` so the hex stays opaque until lowered.
mod opt_lower_hex {
    use serde::{Deserialize, Deserializer, de::Error as _};

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
        let s = Option::<String>::deserialize(d)?;
        if let Some(h) = &s &&
            h.chars().any(|c| c.is_ascii_uppercase())
        {
            return Err(D::Error::custom("expected lowercase hex"));
        }
        Ok(s)
    }
}

/// The target of a [`InstructionSpec::CallMethod`]. Mirrors the engine
/// [`ComponentReference`](tari_ootle_transaction::v1::component_reference::ComponentReference):
/// either an on-ledger address or a component placed on the runtime workspace earlier in the same
/// phase (e.g. an account just created via [`InstructionSpec::CreateAccount`] +
/// [`InstructionSpec::PutLastInstructionOutputOnWorkspace`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ComponentRef {
    /// An on-ledger component address (`component_<hex>`).
    Address(ComponentAddressStr),
    /// A workspace label bound by a prior `PutLastInstructionOutputOnWorkspace` in the same phase.
    Workspace(String),
}

/// The owner rule for a created account. A reduced, closed-set form of the engine
/// [`OwnerRule`](tari_template_lib_types::SubstateOwnerRule); the access-rule arm is omitted to
/// avoid freezing the full access-rule DSL into the ABI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OwnerRuleSpec {
    /// Owned by the transaction signer (the engine default).
    OwnedBySigner,
    /// No owner; only access rules apply.
    None,
    /// Owned by a specific public key.
    ByPublicKey(PublicKeyBytes),
}

/// A single instruction, boundary form. Covers a subset of the engine
/// [`Instruction`](tari_ootle_transaction::Instruction) enum. `ClaimBurn` / `AllocateAddress` / NFT
/// args are omitted to avoid freezing an over-broad surface into the ABI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum InstructionSpec {
    /// Call a template function: `template_address::function(args)`. Maps to
    /// `Instruction::CallFunction`.
    CallFunction {
        /// The template address (`<hex>` template address string).
        template_address: String,
        /// The function name.
        function: String,
        /// The typed arguments (encoded via [`encode_arg`]).
        args: Vec<ArgValue>,
    },
    /// Call a component method. Maps to `Instruction::CallMethod`. The target is a [`ComponentRef`] —
    /// either an address or a workspace component.
    CallMethod {
        /// The component to call (address or workspace label).
        call: ComponentRef,
        /// The method name.
        method: String,
        /// The typed arguments (encoded via [`encode_arg`]).
        args: Vec<ArgValue>,
    },
    /// Idempotent create-or-fetch of an account component for `owner_public_key`. Maps to
    /// `Instruction::CreateAccount`.
    CreateAccount {
        /// The account owner's public key.
        owner_public_key: PublicKeyBytes,
        /// Optional owner rule. `None` uses the engine default (owned by signer).
        #[serde(default)]
        owner_rule: Option<OwnerRuleSpec>,
        /// Optional workspace label of a bucket to deposit into the account atomically on creation.
        #[serde(default)]
        bucket_workspace_id: Option<String>,
    },
    /// Publish a template from the transaction blob at `blob_index` (into [`GenericTransactionIntent::blobs`]).
    /// Maps to `Instruction::PublishTemplate`.
    PublishTemplate {
        /// Index into [`GenericTransactionIntent::blobs`].
        blob_index: u32,
        /// Optional multihash (lowercase hex) of off-chain CBOR metadata.
        #[serde(default, deserialize_with = "opt_lower_hex::deserialize")]
        metadata_hash: Option<String>,
    },
    /// Move the last instruction's output onto the workspace under the label `key`. Maps to
    /// `Instruction::PutLastInstructionOutputOnWorkspace`.
    ///
    /// The label is a string; the engine instruction carries a numeric workspace id assigned by the
    /// builder, not here.
    PutLastInstructionOutputOnWorkspace {
        /// The workspace label this output is stored under (referenced later by
        /// [`ArgValue::Workspace`]).
        key: String,
    },
}

/// The typed argument DSL.
///
/// Each variant maps onto exactly one of [`InstructionArg`]'s carriers via [`encode_arg`]:
///
/// | `ArgValue`                       | `InstructionArg` carrier              |
/// |----------------------------------|---------------------------------------|
/// | `Amount`/`String`/`Bool`/`U64`/`I64`/`Address`/`Metadata`/`NonFungibleId` | `Literal(Bytes)` (`from_type` → `tari_bor::encode`) |
/// | `Bytes`                          | `Literal(Bytes)` — CBOR byte string (not a u8 array) |
/// | `List`/`Optional`                | `Literal(Bytes)` — CBOR array / value-or-null of the lowered elements |
/// | `Workspace`                      | `Workspace(WorkspaceOffsetId)`        |
///
/// There is no `Blob` arm: blobs are referenced structurally via
/// [`InstructionSpec::PublishTemplate`], not as call args.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArgValue {
    /// A µTari token amount, encoded as the engine [`Amount`].
    Amount(u64),
    /// A canonical substate address string of any kind (component, resource, vault, non-fungible,
    /// transaction receipt, template, validator fee pool, utxo, claimed-output tombstone), encoded as
    /// the inner typed engine address. Malformed ⇒ `"PARSE"`.
    Address(String),
    /// A reference to a value previously placed on the runtime workspace, by its label. The numeric
    /// workspace id is builder-stateful and resolved at the call point; encoding it standalone errors
    /// (see [`encode_arg`]).
    Workspace(String),
    /// A UTF-8 string literal.
    String(String),
    /// A boolean literal.
    Bool(bool),
    /// A `u64` literal (stays a native `u64` end-to-end; never a float).
    U64(u64),
    /// A signed integer literal. Covers `i8`..`i64` by CBOR minimal encoding + range-checked decode;
    /// negatives emit a CBOR negative integer.
    I64(i64),
    /// Raw bytes literal (lowercase hex in JSON).
    Bytes(#[serde(serialize_with = "blob_hex::serialize", deserialize_with = "blob_hex::deserialize")] Vec<u8>),
    /// A metadata map, encoded as the engine [`Metadata`] (`BorTag<_, 129>`) with each value lowered as
    /// a list element would be. Templates taking a `Metadata` parameter (resource builders,
    /// `stable_coin::instantiate`) expect this. A nested [`Workspace`](ArgValue::Workspace) ⇒
    /// `"VALIDATION"`.
    Metadata(BTreeMap<String, ArgValue>),
    /// A non-fungible id in canonical string form (`uuid_<hex>` | `str_<text>` | `u32_<n>` | `u64_<n>`),
    /// for template parameters taking the id value itself (distinct from an NFT address). Malformed ⇒
    /// `"PARSE"`.
    NonFungibleId(String),
    /// A list of arguments, encoded as a CBOR array of the lowered elements. Byte-identical to a native
    /// `Vec<T>` for homogeneous elements; templates taking `Vec<T>` parameters (e.g. NFT-mint id lists)
    /// expect this. A nested [`Workspace`](ArgValue::Workspace) ⇒ `"VALIDATION"`.
    List(Vec<ArgValue>),
    /// An optional argument: the lowered inner value, or CBOR null when absent. Byte-identical to a
    /// native `Option<T>`. A nested [`Workspace`](ArgValue::Workspace) ⇒ `"VALIDATION"`.
    Optional(Option<Box<ArgValue>>),
}

/// Encodes one [`ArgValue`] onto the builder's [`InstructionArg`] encoder.
///
/// Literal arms route through [`InstructionArg::from_type`] → `tari_bor::encode` — the same encoder
/// the builder uses — so the produced bytes cannot drift from the engine's wire format.
/// The [`Address`](ArgValue::Address) arm first parses the string into the typed engine address, then
/// encodes that inner typed value, so the encoded literal is the address type, not a bare string.
///
/// # Errors
///
/// - [`ArgValue::Address`] that is not a valid canonical substate address ⇒ [`OotleSdkError::Parse`] (`"PARSE"`).
/// - [`ArgValue::NonFungibleId`] whose string is not a valid canonical non-fungible id (`uuid_<hex>` / `str_<text>` /
///   `u32_<n>` / `u64_<n>`) ⇒ [`OotleSdkError::Parse`] (`"PARSE"`).
/// - [`ArgValue::Workspace`] ⇒ [`OotleSdkError::Validation`] (`"VALIDATION"`): the numeric workspace id is
///   builder-stateful and only resolvable during builder composition, not pre-resolved here.
///
/// Pure: no RNG, no I/O.
pub fn encode_arg(arg: &ArgValue) -> Result<InstructionArg, OotleSdkError> {
    match arg {
        ArgValue::Amount(a) => from_type(&Amount::from(*a)),
        ArgValue::Address(s) => encode_address(s),
        ArgValue::Workspace(label) => Err(OotleSdkError::Validation(format!(
            "workspace arg '{label}' cannot be encoded standalone: its numeric id is assigned during builder \
             composition (resolved in the generic builder, not here)"
        ))),
        ArgValue::String(s) => from_type(s),
        ArgValue::Bool(b) => from_type(b),
        ArgValue::U64(n) => from_type(n),
        ArgValue::I64(n) => from_type(n),
        // Encode as a CBOR byte string (not a u8 array): byte newtypes like RistrettoPublicKeyBytes
        // decode from a definite-length byte string. `from_type(&Vec<u8>)` would emit an array.
        ArgValue::Bytes(bytes) => InstructionArg::literal(tari_bor::Value::Bytes(bytes.clone()))
            .map_err(|e| OotleSdkError::Encoding(format!("arg encode failed: {e}"))),
        ArgValue::Metadata(map) => from_type(&metadata_from_args(map, 0)?),
        ArgValue::NonFungibleId(s) => encode_non_fungible_id(s),
        // Containers lower each element to a `tari_bor::Value`, assemble the CBOR array / value-or-null,
        // then encode the assembled value — byte-identical to a native `Vec<T>` / `Option<T>`.
        ArgValue::List(_) | ArgValue::Optional(_) => {
            let value = arg_to_value(arg, 0)?;
            InstructionArg::literal(value).map_err(|e| OotleSdkError::Encoding(format!("arg encode failed: {e}")))
        },
    }
}

/// Maximum argument nesting depth. A host could send a deeply nested composite; past this limit the
/// argument is rejected with `"VALIDATION"` rather than recursing into a stack overflow.
const MAX_ARG_DEPTH: usize = 16;

/// Lowers any [`ArgValue`] to a [`tari_bor::Value`], the form used when assembling composite args.
///
/// Scalars are encoded through the same typed encoders as [`encode_arg`] (so a list element and the
/// equivalent standalone arg agree by construction), then materialised as a value tree. Containers
/// recurse, assembling a CBOR array ([`List`](ArgValue::List)) or the inner value / null
/// ([`Optional`](ArgValue::Optional)).
///
/// # Errors
///
/// - A nested [`Workspace`](ArgValue::Workspace) ⇒ [`OotleSdkError::Validation`]: its numeric id is only resolvable for
///   a top-level arg during builder composition, never inside a container.
/// - Nesting past [`MAX_ARG_DEPTH`] ⇒ [`OotleSdkError::Validation`].
/// - A malformed [`Address`](ArgValue::Address) / [`NonFungibleId`](ArgValue::NonFungibleId) ⇒
///   [`OotleSdkError::Parse`].
fn arg_to_value(arg: &ArgValue, depth: usize) -> Result<tari_bor::Value, OotleSdkError> {
    if depth > MAX_ARG_DEPTH {
        return Err(OotleSdkError::Validation("argument nesting too deep".to_string()));
    }
    match arg {
        ArgValue::Amount(a) => to_value(&Amount::from(*a)),
        ArgValue::Address(s) => address_to_value(s),
        ArgValue::String(s) => to_value(s),
        ArgValue::Bool(b) => to_value(b),
        ArgValue::U64(n) => to_value(n),
        ArgValue::I64(n) => to_value(n),
        ArgValue::Bytes(bytes) => Ok(tari_bor::Value::Bytes(bytes.clone())),
        ArgValue::Metadata(map) => to_value(&metadata_from_args(map, depth)?),
        ArgValue::NonFungibleId(s) => non_fungible_id_to_value(s),
        ArgValue::List(items) => {
            let values = items
                .iter()
                .map(|item| arg_to_value(item, depth + 1))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(tari_bor::Value::Array(values))
        },
        ArgValue::Optional(None) => Ok(tari_bor::Value::Null),
        ArgValue::Optional(Some(inner)) => arg_to_value(inner, depth + 1),
        ArgValue::Workspace(label) => Err(OotleSdkError::Validation(format!(
            "workspace arg '{label}' cannot appear inside a composite argument: its numeric id is assigned during \
             builder composition and only resolvable for a top-level arg"
        ))),
    }
}

/// Builds the engine [`Metadata`] for an [`ArgValue::Metadata`] map at `depth`, each value lowered by
/// [`arg_to_value`] one level down.
fn metadata_from_args(map: &BTreeMap<String, ArgValue>, depth: usize) -> Result<Metadata, OotleSdkError> {
    let mut metadata = Metadata::new();
    for (key, arg) in map {
        let value = arg_to_value(arg, depth + 1)?;
        let raw = tari_bor::RawCbor::from_value(&value)
            .map_err(|e| OotleSdkError::Encoding(format!("arg encode failed: {e}")))?;
        metadata.insert_raw(key.clone(), raw);
    }
    Ok(metadata)
}

/// Builds the workspace-reference carrier for a resolved numeric id.
///
/// The builder composition assigns the numeric [`WorkspaceOffsetId`] for an [`ArgValue::Workspace`]
/// label, then calls this to produce the carrier. Kept here so the one
/// `ArgValue::Workspace → InstructionArg::Workspace` mapping lives alongside the rest of the DSL, even
/// though the id itself is builder-stateful.
pub fn workspace_arg(id: WorkspaceOffsetId) -> InstructionArg {
    InstructionArg::workspace_offset(id)
}

/// Encodes a canonical substate address string as its inner typed engine address.
///
/// Dispatches on the address prefix to recover the typed inner address (component, resource, vault,
/// non-fungible, transaction receipt, template, validator fee pool, utxo, or claimed-output
/// tombstone), then `from_type`s that inner value — never the wrapping enum. The encoded literal is
/// therefore the address type a template parameter expects. An unrecognized or malformed string ⇒
/// [`OotleSdkError::Parse`].
fn encode_address(s: &str) -> Result<InstructionArg, OotleSdkError> {
    let id = parse_address(s)?;
    match id {
        SubstateId::Component(a) => from_type(&a),
        SubstateId::Resource(a) => from_type(&a),
        SubstateId::Vault(a) => from_type(&a),
        SubstateId::NonFungible(a) => from_type(&a),
        SubstateId::TransactionReceipt(a) => from_type(&a),
        SubstateId::Template(a) => from_type(&a.as_template_address()),
        SubstateId::ValidatorFeePool(a) => from_type(&a),
        SubstateId::Utxo(a) => from_type(&a),
        SubstateId::ConfidentialOutput(a) => from_type(&a),
        SubstateId::ClaimedOutputTombstone(a) => from_type(&a),
    }
}

/// Lowers a canonical substate address string to the value tree of its inner typed engine address.
///
/// Shares the prefix dispatch with [`encode_address`] so a list element and the equivalent standalone
/// address arg encode identically. Malformed ⇒ [`OotleSdkError::Parse`].
fn address_to_value(s: &str) -> Result<tari_bor::Value, OotleSdkError> {
    let id = parse_address(s)?;
    match id {
        SubstateId::Component(a) => to_value(&a),
        SubstateId::Resource(a) => to_value(&a),
        SubstateId::Vault(a) => to_value(&a),
        SubstateId::NonFungible(a) => to_value(&a),
        SubstateId::TransactionReceipt(a) => to_value(&a),
        SubstateId::Template(a) => to_value(&a.as_template_address()),
        SubstateId::ValidatorFeePool(a) => to_value(&a),
        SubstateId::Utxo(a) => to_value(&a),
        SubstateId::ConfidentialOutput(a) => to_value(&a),
        SubstateId::ClaimedOutputTombstone(a) => to_value(&a),
    }
}

/// Parses a canonical substate address string into its typed [`SubstateId`]; malformed ⇒
/// [`OotleSdkError::Parse`].
fn parse_address(s: &str) -> Result<SubstateId, OotleSdkError> {
    SubstateId::from_str(s).map_err(|e| OotleSdkError::Parse(format!("address arg '{s}': {e}")))
}

/// Encodes a canonical non-fungible id string as the typed engine id literal.
fn encode_non_fungible_id(s: &str) -> Result<InstructionArg, OotleSdkError> {
    from_type(&parse_non_fungible_id(s)?)
}

/// Lowers a canonical non-fungible id string to its typed engine id value tree (shares the parse path
/// with [`encode_non_fungible_id`]). Malformed ⇒ [`OotleSdkError::Parse`].
fn non_fungible_id_to_value(s: &str) -> Result<tari_bor::Value, OotleSdkError> {
    to_value(&parse_non_fungible_id(s)?)
}

/// Parses a canonical non-fungible id string (`uuid_<hex>` / `str_<text>` / `u32_<n>` / `u64_<n>`) via
/// the fallible parser, never the panicking form; malformed ⇒ [`OotleSdkError::Parse`].
fn parse_non_fungible_id(s: &str) -> Result<tari_template_lib_types::NonFungibleId, OotleSdkError> {
    tari_template_lib_types::NonFungibleId::try_from_canonical_string(s)
        .map_err(|e| OotleSdkError::Parse(format!("invalid non-fungible id '{s}': {e}")))
}

/// Bridges a `tari_bor::Encode + Serialize` value through the builder's literal encoder, mapping any
/// encode failure onto the stable `"ENCODING"` code.
fn from_type<T: Serialize + tari_bor::Encode<()>>(val: &T) -> Result<InstructionArg, OotleSdkError> {
    InstructionArg::from_type(val).map_err(|e| OotleSdkError::Encoding(format!("arg encode failed: {e}")))
}

/// Bridges a value into its [`tari_bor::Value`] tree, mapping any encode failure onto the stable
/// `"ENCODING"` code. Used when assembling composite args.
fn to_value<T: tari_bor::Encode<()> + ?Sized>(val: &T) -> Result<tari_bor::Value, OotleSdkError> {
    tari_bor::to_value(val).map_err(|e| OotleSdkError::Encoding(format!("arg encode failed: {e}")))
}

/// Where the transaction fee is charged from. Mirrors the engine's fee-payment surface so the
/// generic entry point can express a self-funding transaction (one that pays the fee from value
/// produced within the same transaction), not just an existing account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FeeSource {
    /// Pay from an existing on-ledger account's TARI vault. Derives a required vault want.
    FromAccount(ComponentAddressStr),
    /// Pay from a component placed on the workspace earlier in the fee phase (e.g. a just-created
    /// account). The label is bound by a `PutLastInstructionOutputOnWorkspace` in `fee_instructions`.
    /// Derives no vault want — the component is not on-ledger yet.
    FromWorkspaceComponent {
        /// The workspace label of the paying component.
        label: String,
    },
    /// Pay from a bucket on the workspace (`Instruction::PayFeeFromBucket`). Overpayment is not
    /// refunded. The label is bound by a `PutLastInstructionOutputOnWorkspace` in `fee_instructions`.
    FromBucket {
        /// The workspace label of the bucket.
        label: String,
    },
}

/// A generic transaction intent: an explicit instruction list + typed args + blobs + inputs.
///
/// The front-end that replaces the hardcoded intents. The resolution/seal pipeline that consumes it
/// produces the same `PartialTransaction` + `WantList` as the public-transfer path; this type only
/// defines the boundary record.
///
/// Instructions are split across the engine's two execution phases: `fee_instructions` run first
/// (the fee is charged at the end of this phase), then the workspace is cleared, then `instructions`
/// run. Self-funding (create + fund + pay from the new account) lives entirely in `fee_instructions`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenericTransactionIntent {
    /// The fee to pay, in µTari.
    pub fee: BoundaryAmount,
    /// Where the fee is charged from.
    pub fee_payment: FeeSource,
    /// Fee-phase instructions, in order. Run before `instructions`; the fee is charged at the end of
    /// this phase. Self-funding setup (create account, fund it) goes here. Empty for the common case.
    pub fee_instructions: Vec<InstructionSpec>,
    /// Main-phase instruction list, in order.
    pub instructions: Vec<InstructionSpec>,
    /// Transaction blobs (e.g. WASM binaries for `publish_template`), referenced by index.
    pub blobs: Vec<BlobSpec>,
    /// The explicit input set. Empty ⇒ want-list resolution.
    pub inputs: Vec<InputRef>,
    /// Inputs to pin **in addition to** the derived/explicit set. Unlike `inputs` (which is
    /// all-or-nothing: non-empty short-circuits derivation), these are always merged in. For inputs a
    /// template references internally and the instruction list can't reveal — e.g. a faucet's claim
    /// resource. Empty for the common case.
    #[serde(default)]
    pub extra_inputs: Vec<InputRef>,
    /// Optional earliest epoch this transaction is valid in.
    pub min_epoch: Option<u64>,
    /// The last epoch this transaction is valid in. Mandatory: every transaction has a bounded
    /// validity window, capped network-wide at `max_transaction_validity_epochs` past the current epoch.
    pub max_epoch: u64,
    /// Whether this is a dry run.
    pub dry_run: bool,
}

impl GenericTransactionIntent {
    /// Validates that every [`InstructionSpec::PublishTemplate`] (in either phase) references an
    /// in-range blob index.
    ///
    /// A blob index `>= blobs.len()` ⇒ [`OotleSdkError::Validation`] (`"VALIDATION"`). This is the
    /// cheap structural check the boundary can make standalone, before the full instruction → builder
    /// lowering.
    pub fn validate_blob_indices(&self) -> Result<(), OotleSdkError> {
        let n = self.blobs.len();
        for instr in self.fee_instructions.iter().chain(&self.instructions) {
            if let InstructionSpec::PublishTemplate { blob_index, .. } = instr &&
                (*blob_index as usize) >= n
            {
                return Err(OotleSdkError::Validation(format!(
                    "publish_template references blob index {blob_index} but only {n} blob(s) are present"
                )));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use tari_template_lib_types::{
        ClaimedOutputTombstoneAddress,
        ComponentAddress,
        Hash32,
        NonFungibleAddress,
        NonFungibleId,
        ObjectKey,
        ResourceAddress,
        TransactionReceiptAddress,
        UtxoAddress,
        UtxoId,
        ValidatorFeePoolAddress,
        VaultId,
        crypto::RistrettoPublicKeyBytes,
    };

    use super::*;

    fn component_str() -> String {
        ComponentAddress::new(ObjectKey::from_array([0xaa; ObjectKey::LENGTH])).to_string()
    }

    fn resource_str() -> String {
        ResourceAddress::new(ObjectKey::from_array([0xbb; ObjectKey::LENGTH])).to_string()
    }

    fn sample_resource() -> ResourceAddress {
        ResourceAddress::new(ObjectKey::from_array([0xcc; ObjectKey::LENGTH]))
    }

    /// The encoded literal bytes (or `None` for a non-literal carrier).
    fn literal_bytes(arg: &InstructionArg) -> Option<Vec<u8>> {
        arg.as_literal_bytes().map(<[u8]>::to_vec)
    }

    /// The builder's native literal encoder (`InstructionArg::from_type` → `tari_bor::encode`).
    /// Asserting equality against this proves the DSL produces byte-identical literals to the builder.
    fn native<T: Serialize + tari_bor::Encode<()>>(val: &T) -> InstructionArg {
        InstructionArg::from_type(val).unwrap()
    }

    #[test]
    fn amount_encodes_identically_to_the_builder() {
        let dsl = encode_arg(&ArgValue::Amount(1_000_000)).unwrap();
        assert_eq!(
            dsl,
            native(&Amount::from(1_000_000u64)),
            "Amount must encode byte-identically to the builder's Amount literal"
        );
    }

    #[test]
    fn amount_survives_above_2_pow_33() {
        // A value > 2^33 proves no float truncation on the amount path.
        let v: u64 = (1u64 << 34) + 7;
        let dsl = encode_arg(&ArgValue::Amount(v)).unwrap();
        assert_eq!(dsl, native(&Amount::from(v)));
    }

    #[test]
    fn string_encodes_identically_to_the_builder() {
        let dsl = encode_arg(&ArgValue::String("withdraw".to_string())).unwrap();
        assert_eq!(dsl, native(&"withdraw".to_string()));
    }

    #[test]
    fn bool_encodes_identically_to_the_builder() {
        let dsl = encode_arg(&ArgValue::Bool(true)).unwrap();
        assert_eq!(dsl, native(&true));
    }

    #[test]
    fn u64_encodes_identically_to_the_builder() {
        let v: u64 = (1u64 << 34) + 99;
        let dsl = encode_arg(&ArgValue::U64(v)).unwrap();
        assert_eq!(dsl, native(&v));
    }

    #[test]
    fn i64_positive_encodes_identically_to_the_builder() {
        let dsl = encode_arg(&ArgValue::I64(5)).unwrap();
        assert_eq!(
            dsl,
            native(&5i64),
            "positive I64 must encode byte-identically to the builder"
        );
    }

    #[test]
    fn i64_negative_encodes_identically_to_the_builder() {
        let dsl = encode_arg(&ArgValue::I64(-5)).unwrap();
        assert_eq!(
            dsl,
            native(&-5i64),
            "negative I64 must encode byte-identically to the builder"
        );
    }

    #[test]
    fn i64_min_encodes_identically_to_the_builder() {
        let dsl = encode_arg(&ArgValue::I64(i64::MIN)).unwrap();
        assert_eq!(
            dsl,
            native(&i64::MIN),
            "i64::MIN must encode byte-identically to the builder"
        );
    }

    #[test]
    fn u64_literal_decodes_into_a_narrow_unsigned_field() {
        // A single U64 arm satisfies any narrower unsigned param: its minimal CBOR literal decodes
        // straight into a u32, so no separate U8/U16/U32 arm is needed.
        let dsl = encode_arg(&ArgValue::U64(5)).unwrap();
        let bytes = literal_bytes(&dsl).unwrap();
        let decoded: u32 = tari_bor::decode(&bytes).unwrap();
        assert_eq!(decoded, 5);
    }

    #[test]
    fn bytes_encodes_as_a_cbor_byte_string() {
        // The Bytes arm emits a definite-length CBOR byte string (so byte newtypes decode from it),
        // not a u8 array as `from_type(&Vec<u8>)` would.
        let raw = vec![0xde, 0xad, 0xbe, 0xef];
        let dsl = encode_arg(&ArgValue::Bytes(raw.clone())).unwrap();
        let expected = InstructionArg::literal(tari_bor::Value::Bytes(raw)).unwrap();
        assert_eq!(dsl, expected, "Bytes must encode as a CBOR byte string");
    }

    #[test]
    fn non_fungible_id_u256_encodes_identically_to_the_builder() {
        let id = NonFungibleId::from_u256([0x5a; 32]);
        let dsl = encode_arg(&ArgValue::NonFungibleId(id.to_canonical_string())).unwrap();
        assert_eq!(
            dsl,
            native(&id),
            "uuid non-fungible id must encode byte-identically to the builder"
        );
    }

    #[test]
    fn non_fungible_id_string_encodes_identically_to_the_builder() {
        let id = NonFungibleId::try_from_string("special-nft").unwrap();
        let dsl = encode_arg(&ArgValue::NonFungibleId(id.to_canonical_string())).unwrap();
        assert_eq!(
            dsl,
            native(&id),
            "str non-fungible id must encode byte-identically to the builder"
        );
    }

    #[test]
    fn non_fungible_id_u32_encodes_identically_to_the_builder() {
        let id = NonFungibleId::from_u32(7);
        let dsl = encode_arg(&ArgValue::NonFungibleId(id.to_canonical_string())).unwrap();
        assert_eq!(
            dsl,
            native(&id),
            "u32 non-fungible id must encode byte-identically to the builder"
        );
    }

    #[test]
    fn non_fungible_id_u64_encodes_identically_to_the_builder() {
        let id = NonFungibleId::from_u64((1u64 << 34) + 5);
        let dsl = encode_arg(&ArgValue::NonFungibleId(id.to_canonical_string())).unwrap();
        assert_eq!(
            dsl,
            native(&id),
            "u64 non-fungible id must encode byte-identically to the builder"
        );
    }

    #[test]
    fn malformed_non_fungible_id_is_a_parse_error_without_panic() {
        // A garbage string and an over-64-char `str_…` value both error (no panic).
        let garbage = encode_arg(&ArgValue::NonFungibleId("not-a-canonical-id".to_string())).unwrap_err();
        assert_eq!(garbage.code(), "PARSE");

        let over_length = format!("str_{}", "a".repeat(65));
        let too_long = encode_arg(&ArgValue::NonFungibleId(over_length)).unwrap_err();
        assert_eq!(too_long.code(), "PARSE");
    }

    #[test]
    fn component_address_encodes_as_the_typed_address() {
        let s = component_str();
        let dsl = encode_arg(&ArgValue::Address(s.clone())).unwrap();
        let component: ComponentAddress = s.parse().unwrap();
        assert_eq!(
            dsl,
            native(&component),
            "component address must encode as the typed engine ComponentAddress"
        );
    }

    #[test]
    fn resource_address_encodes_as_the_typed_address() {
        let s = resource_str();
        let dsl = encode_arg(&ArgValue::Address(s.clone())).unwrap();
        let resource: ResourceAddress = s.parse().unwrap();
        assert_eq!(
            dsl,
            native(&resource),
            "resource address must encode as the typed engine ResourceAddress"
        );
    }

    #[test]
    fn vault_address_encodes_as_the_typed_address() {
        let vault = VaultId::new(ObjectKey::from_array([0xdd; ObjectKey::LENGTH]));
        let s = SubstateId::from(vault).to_string();
        let dsl = encode_arg(&ArgValue::Address(s)).unwrap();
        assert_eq!(
            dsl,
            native(&vault),
            "vault address must encode as the typed engine VaultId"
        );
    }

    #[test]
    fn non_fungible_address_encodes_as_the_typed_address() {
        let nft = NonFungibleAddress::new(sample_resource(), NonFungibleId::try_from_string("nft-1").unwrap());
        let s = SubstateId::from(nft.clone()).to_string();
        let dsl = encode_arg(&ArgValue::Address(s)).unwrap();
        assert_eq!(
            dsl,
            native(&nft),
            "non-fungible address must encode as the typed engine NonFungibleAddress"
        );
    }

    #[test]
    fn transaction_receipt_address_encodes_as_the_typed_address() {
        let addr = TransactionReceiptAddress::from_array([0xee; ObjectKey::LENGTH]);
        let s = SubstateId::from(addr).to_string();
        let dsl = encode_arg(&ArgValue::Address(s)).unwrap();
        assert_eq!(dsl, native(&addr));
    }

    #[test]
    fn template_address_encodes_as_a_template_address() {
        use tari_engine_types::published_template::PublishedTemplateAddress;
        let addr = PublishedTemplateAddress::from_hash(Hash32::from_array([0x12; Hash32::LENGTH]));
        let s = SubstateId::from(addr).to_string();
        let dsl = encode_arg(&ArgValue::Address(s)).unwrap();
        assert_eq!(dsl, native(&addr.as_template_address()));
    }

    #[test]
    fn validator_fee_pool_address_encodes_as_the_typed_address() {
        let addr = ValidatorFeePoolAddress::from_array([0x34; ObjectKey::LENGTH]);
        let s = SubstateId::from(addr).to_string();
        let dsl = encode_arg(&ArgValue::Address(s)).unwrap();
        assert_eq!(dsl, native(&addr));
    }

    #[test]
    fn utxo_address_encodes_as_the_typed_address() {
        let addr = UtxoAddress::new(sample_resource(), UtxoId::from_array([0x56; UtxoId::LENGTH]));
        let s = SubstateId::from(addr.clone()).to_string();
        let dsl = encode_arg(&ArgValue::Address(s)).unwrap();
        assert_eq!(dsl, native(&addr));
    }

    #[test]
    fn claimed_output_tombstone_address_encodes_as_the_typed_address() {
        let addr = ClaimedOutputTombstoneAddress::new(ObjectKey::from_array([0x78; ObjectKey::LENGTH]));
        let s = SubstateId::from(addr).to_string();
        let dsl = encode_arg(&ArgValue::Address(s)).unwrap();
        assert_eq!(dsl, native(&addr));
    }

    #[test]
    fn malformed_address_is_a_parse_error() {
        let err = encode_arg(&ArgValue::Address("not-an-address".to_string())).unwrap_err();
        assert_eq!(err.code(), "PARSE");
    }

    #[test]
    fn unknown_address_prefix_is_a_parse_error_without_panic() {
        // A well-formed `<prefix>_<hex>` shape whose prefix is not a substate kind must error, not panic.
        let err = encode_arg(&ArgValue::Address("bogus_aabbccdd".to_string())).unwrap_err();
        assert_eq!(err.code(), "PARSE");
    }

    #[test]
    fn workspace_arg_is_not_encodable_standalone() {
        let err = encode_arg(&ArgValue::Workspace("bucket".to_string())).unwrap_err();
        assert_eq!(err.code(), "VALIDATION");
    }

    #[test]
    fn workspace_arg_helper_produces_the_workspace_carrier() {
        let id = WorkspaceOffsetId::new(3);
        let arg = workspace_arg(id);
        assert_eq!(arg, InstructionArg::workspace(3, None));
        assert!(literal_bytes(&arg).is_none());
    }

    #[test]
    fn full_withdraw_arg_list_matches_the_builder() {
        // The public-transfer withdraw call's args are `[resource, amount]`. Prove the DSL produces
        // the identical `Vec<InstructionArg>` the builder's encoder does.
        let resource = resource_str();
        let amount: u64 = 5_000_000;
        let dsl: Vec<InstructionArg> = [ArgValue::Address(resource.clone()), ArgValue::Amount(amount)]
            .iter()
            .map(|a| encode_arg(a).unwrap())
            .collect();

        let resource_typed: ResourceAddress = resource.parse().unwrap();
        let expected = vec![native(&resource_typed), native(&Amount::from(amount))];
        assert_eq!(
            dsl, expected,
            "the DSL arg list must equal the builder's native arg list"
        );
    }

    #[test]
    fn list_of_u64_encodes_like_native_vec() {
        let dsl = encode_arg(&ArgValue::List(vec![
            ArgValue::U64(1),
            ArgValue::U64(2),
            ArgValue::U64(3),
        ]))
        .unwrap();
        assert_eq!(
            dsl,
            native(&vec![1u64, 2u64, 3u64]),
            "a List of u64 must encode byte-identically to a native Vec<u64>"
        );
    }

    #[test]
    fn empty_list_encodes_like_native_empty_vec() {
        let dsl = encode_arg(&ArgValue::List(vec![])).unwrap();
        assert_eq!(
            dsl,
            native(&Vec::<u64>::new()),
            "an empty List must encode as an empty CBOR array"
        );
    }

    #[test]
    fn list_of_addresses_encodes_like_native_vec() {
        let component: ComponentAddress = component_str().parse().unwrap();
        let resource: ResourceAddress = resource_str().parse().unwrap();
        let dsl = encode_arg(&ArgValue::List(vec![
            ArgValue::Address(component_str()),
            ArgValue::Address(resource_str()),
        ]))
        .unwrap();
        // A heterogeneous list still lowers each element via its own typed encoder; the native side
        // builds the equivalent CBOR array of those same encodings.
        let elements = vec![
            tari_bor::to_value(&component).unwrap(),
            tari_bor::to_value(&resource).unwrap(),
        ];
        let expected = native(&tari_bor::Value::Array(elements));
        assert_eq!(
            dsl, expected,
            "a List of addresses must lower each element via its typed-address encoder"
        );
    }

    #[test]
    fn list_of_non_fungible_ids_encodes_like_native_vec() {
        let a = NonFungibleId::from_u32(7);
        let b = NonFungibleId::from_u64((1u64 << 34) + 5);
        let dsl = encode_arg(&ArgValue::List(vec![
            ArgValue::NonFungibleId(a.to_canonical_string()),
            ArgValue::NonFungibleId(b.to_canonical_string()),
        ]))
        .unwrap();
        assert_eq!(
            dsl,
            native(&vec![a, b]),
            "a Vec<NonFungibleId> must encode byte-identically to the builder"
        );
    }

    #[test]
    fn optional_some_encodes_like_native_some() {
        let dsl = encode_arg(&ArgValue::Optional(Some(Box::new(ArgValue::U64(7))))).unwrap();
        assert_eq!(
            dsl,
            native(&Some(7u64)),
            "Optional(Some(x)) must encode byte-identically to a native Some(x)"
        );
    }

    #[test]
    fn optional_none_encodes_like_native_none() {
        let dsl = encode_arg(&ArgValue::Optional(None)).unwrap();
        assert_eq!(
            dsl,
            native(&Option::<u64>::None),
            "Optional(None) must encode as CBOR null, like a native None"
        );
    }

    #[test]
    fn nested_list_of_optionals_encodes_like_native() {
        let dsl = encode_arg(&ArgValue::List(vec![
            ArgValue::Optional(Some(Box::new(ArgValue::U64(1)))),
            ArgValue::Optional(None),
        ]))
        .unwrap();
        assert_eq!(
            dsl,
            native(&vec![Some(1u64), None]),
            "a List of Optionals must encode like a native Vec<Option<u64>>"
        );
    }

    #[test]
    fn nested_list_of_lists_encodes_like_native() {
        let dsl = encode_arg(&ArgValue::List(vec![
            ArgValue::List(vec![ArgValue::U64(1), ArgValue::U64(2)]),
            ArgValue::List(vec![ArgValue::U64(3)]),
        ]))
        .unwrap();
        assert_eq!(
            dsl,
            native(&vec![vec![1u64, 2u64], vec![3u64]]),
            "a List of Lists must encode like a native Vec<Vec<u64>>"
        );
    }

    #[test]
    fn nested_workspace_in_list_is_a_validation_error_without_panic() {
        let err = encode_arg(&ArgValue::List(vec![ArgValue::Workspace("b".to_string())])).unwrap_err();
        assert_eq!(err.code(), "VALIDATION");
    }

    #[test]
    fn nested_workspace_in_optional_is_a_validation_error_without_panic() {
        let err = encode_arg(&ArgValue::Optional(Some(Box::new(ArgValue::Workspace(
            "b".to_string(),
        )))))
        .unwrap_err();
        assert_eq!(err.code(), "VALIDATION");
    }

    #[test]
    fn over_deep_nesting_is_a_validation_error_without_panic() {
        // Wrap a scalar in more than MAX_ARG_DEPTH lists; the bound rejects it as VALIDATION rather
        // than recursing into a stack overflow.
        let mut arg = ArgValue::U64(1);
        for _ in 0..(MAX_ARG_DEPTH + 2) {
            arg = ArgValue::List(vec![arg]);
        }
        let err = encode_arg(&arg).unwrap_err();
        assert_eq!(err.code(), "VALIDATION");
    }

    fn from_account(addr: String) -> FeeSource {
        FeeSource::FromAccount(ComponentAddressStr::parse(addr).unwrap())
    }

    #[test]
    fn blob_index_bounds_are_validated() {
        let intent = GenericTransactionIntent {
            fee: BoundaryAmount::new(2000),
            fee_payment: from_account(component_str()),
            fee_instructions: vec![],
            instructions: vec![InstructionSpec::PublishTemplate {
                blob_index: 1,
                metadata_hash: None,
            }],
            blobs: vec![BlobSpec { bytes: vec![1, 2, 3] }],
            inputs: vec![],
            extra_inputs: vec![],
            min_epoch: None,
            max_epoch: 1,
            dry_run: false,
        };
        assert_eq!(intent.validate_blob_indices().unwrap_err().code(), "VALIDATION");

        let ok = GenericTransactionIntent {
            instructions: vec![InstructionSpec::PublishTemplate {
                blob_index: 0,
                metadata_hash: None,
            }],
            ..intent
        };
        assert!(ok.validate_blob_indices().is_ok());
    }

    #[test]
    fn intent_serde_round_trips() {
        let intent = GenericTransactionIntent {
            fee: BoundaryAmount::new(2500),
            fee_payment: from_account(component_str()),
            fee_instructions: vec![InstructionSpec::CreateAccount {
                owner_public_key: PublicKeyBytes::from_array([9u8; 32]),
                owner_rule: Some(OwnerRuleSpec::None),
                bucket_workspace_id: Some("seed".to_string()),
            }],
            instructions: vec![
                InstructionSpec::CallMethod {
                    call: ComponentRef::Address(ComponentAddressStr::parse(component_str()).unwrap()),
                    method: "withdraw".to_string(),
                    args: vec![ArgValue::Address(resource_str()), ArgValue::Amount(1_000_000)],
                },
                InstructionSpec::PutLastInstructionOutputOnWorkspace {
                    key: "bucket0".to_string(),
                },
                InstructionSpec::CreateAccount {
                    owner_public_key: PublicKeyBytes::from_array([7u8; 32]),
                    owner_rule: None,
                    bucket_workspace_id: None,
                },
                InstructionSpec::PublishTemplate {
                    blob_index: 0,
                    metadata_hash: Some("12200102".to_string()),
                },
                InstructionSpec::CallFunction {
                    template_address: RistrettoPublicKeyBytes::from([0u8; 32]).to_string(),
                    function: "take_free_coins".to_string(),
                    args: vec![
                        ArgValue::Bool(true),
                        ArgValue::U64(42),
                        ArgValue::Workspace("b".to_string()),
                    ],
                },
            ],
            blobs: vec![BlobSpec {
                bytes: vec![0xaa, 0xbb],
            }],
            inputs: vec![InputRef::versioned(component_str(), 0)],
            extra_inputs: vec![],
            min_epoch: Some(1),
            max_epoch: 10,
            dry_run: true,
        };
        let json = serde_json::to_string(&intent).unwrap();
        let back: GenericTransactionIntent = serde_json::from_str(&json).unwrap();
        assert_eq!(back, intent);
    }

    #[test]
    fn bytes_serde_is_lowercase_hex() {
        let arg = ArgValue::Bytes(vec![0xde, 0xad, 0xbe, 0xef]);
        let json = serde_json::to_string(&arg).unwrap();
        assert_eq!(json, r#"{"Bytes":"deadbeef"}"#);
        // Uppercase hex must be rejected on deserialize (matches the byte newtypes' discipline).
        assert!(serde_json::from_str::<ArgValue>(r#"{"Bytes":"DEAD"}"#).is_err());
    }

    #[test]
    fn metadata_hash_rejects_uppercase_hex() {
        // Lowercase round-trips; uppercase is rejected at the boundary, like every other hex field.
        let ok = r#"{"PublishTemplate":{"blob_index":0,"metadata_hash":"12200102"}}"#;
        assert!(serde_json::from_str::<InstructionSpec>(ok).is_ok());
        let bad = r#"{"PublishTemplate":{"blob_index":0,"metadata_hash":"1220AB"}}"#;
        assert!(serde_json::from_str::<InstructionSpec>(bad).is_err());
    }
}
