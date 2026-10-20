//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Golden-vector harness: the language-neutral fixture **format**, a **generator** that drives the
//! core to produce the `expected` bytes, and the loader/comparison helpers the **runner** uses to
//! assert the core reproduces those bytes byte-for-byte.
//!
//! Why this lives in `tests/` and not `src/`: the generator shells out to `git rev-parse HEAD` and
//! reads environment variables. That is fine for test tooling but would violate the crate's purity
//! rule (no clock / process / env in `src/`). So the harness is test-only support code.
//!
//! ## Format (one JSON file per vector, `fixtures/<group>/<name>.json`)
//!
//! ```json
//! {
//!   "name": "sample/...",
//!   "schema_version": 1,
//!   "provenance": { "core_version": "...", "git_rev": "...", "generated_by": "..." },
//!   "operation": "build_and_encode_public_transfer",
//!   "input":   { ...fully-deterministic arguments to the core function... },
//!   "expected":{ "encoded_transaction": "<lowercase hex>", "transaction_id": "<lowercase hex>" }
//! }
//! ```
//!
//! - **bytes are lowercase hex, no `0x` prefix** (mirrors the Python SDK fixtures). The byte newtypes in
//!   [`ootle_sdk_core::types::bytes`] already serialize this way; `expected` is just their hex.
//! - **`expected` is generator-owned** — never hand-edited. The generator is the only writer.
//! - **the runner compares hex strings, not parsed structures** — that is the whole point: a parsed compare would hide
//!   CBOR/encoding drift.
//! - **pinned nonce secrets are part of `input`**: without them neither `encoded_transaction` nor `transaction_id` is
//!   reproducible.

use std::{fs, path::PathBuf, process::Command};

use ootle_sdk_core::{
    ArgValue,
    FetchedSubstate,
    Resolution,
    StealthKeys,
    account_balances,
    add_signature,
    apply_fetched_substates,
    build_and_encode_public_transfer,
    build_and_encode_stealth_transfer_with_seed,
    build_faucet_claim_with_wants,
    build_public_transfer_unsigned_with_wants,
    build_unsigned_instructions_with_wants,
    decode_stealth_utxo,
    decode_substate,
    derive_account_address,
    derive_account_keypair_from_seed,
    derive_view_keypair_from_seed,
    encode_arg,
    format_identity_address,
    keys::PublicTransferKeys,
    parse_address,
    parse_finalized_result,
    seal_and_encode_public_transfer,
    seal_and_encode_with_auth,
    types::{
        bytes::{BuildSeed, PublicKeyBytes, SecretKeyBytes},
        intent::PublicTransferIntent,
        network::Network,
    },
};
use serde::{Deserialize, Serialize};

/// The current fixture-format schema version. Bump when the JSON shape changes.
pub const SCHEMA_VERSION: u32 = 1;

/// The canonical operation id for the headline public-transfer entry point.
pub const OP_BUILD_AND_ENCODE_PUBLIC_TRANSFER: &str = "build_and_encode_public_transfer";

/// The canonical operation id for the **resolved-path** one-call convenience: build with wants →
/// apply one fetched batch → seal/encode. Its `input.fetched` carries the substates.
pub const OP_RESOLVE_AND_ENCODE_PUBLIC_TRANSFER: &str = "resolve_and_encode_public_transfer";

/// The canonical operation id for **result parsing**: parse a raw indexer finalized-result
/// JSON into the typed [`ootle_sdk_core::types::result::FinalizedResult`].
///
/// **This vector type is different from the encode vectors above.** Encode vectors assert
/// **byte-for-byte** on hex; a parse vector instead **compares a parsed structure**. Its `input.raw_result`
/// carries the raw indexer JSON verbatim (committed as-is), and its `expected.parsed` carries the
/// canonical JSON of the `FinalizedResult` the core produces. The runner compares **canonicalized**
/// JSON (object keys sorted) to avoid key-order noise — there is no CBOR byte stream to drift here, so a
/// structural compare is correct (and a byte compare would be brittle against JSON key ordering).
pub const OP_PARSE_FINALIZED_RESULT: &str = "parse_finalized_result";

/// The canonical operation id for the **stealth outputs statement** build. Its `input`
/// carries a [`ootle_sdk_core::stealth::StealthTransferIntent`] (`stealth_intent`) plus a pinned
/// [`ootle_sdk_core::stealth::StealthEntropy`] (`stealth_entropy`).
///
/// **This vector uses the `"semantic"` comparison mode.**
/// The aggregated bulletproof (`agg_range_proof`) is byte-unstable across runs (the prover's internal
/// `SysRng` blinds the final scalars), so a byte compare is impossible. The runner instead (1)
/// validates the statement cryptographically via `validate_stealth_outputs_statement`, and (2)
/// compares the **deterministic** fields structurally (everything except `agg_range_proof`, which the
/// generator nulls out before recording `expected.stealth_outputs_statement`).
pub const OP_BUILD_STEALTH_OUTPUTS_STATEMENT: &str = "build_stealth_outputs_statement";

/// The canonical operation id for the **full stealth send**: build →
/// sign/seal/encode the confidential transfer into submit-ready BOR bytes. Its `input` carries a
/// `stealth_intent` + pinned `stealth_entropy` + `stealth_keys` (+ optional `fetched`/`spend_secrets`
/// for the stealth-input cases).
///
/// **This vector uses the `"semantic"` comparison mode.**
/// The embedded aggregated bulletproof + balance-proof signature are byte-unstable (no injectable
/// nonce seam in the reused crypto leaves), and the seal/authorization signatures sign a digest that
/// commits to those proofs, so the final sealed bytes/id are not reproducible. The runner instead (1)
/// re-validates every signature on the freshly sealed transaction, and (2) compares the **decoded**
/// transaction structurally with the byte-unstable fields (proofs + signature scalars) nulled.
pub const OP_BUILD_AND_ENCODE_STEALTH_TRANSFER: &str = "build_and_encode_stealth_transfer";

/// The canonical operation id for the **stealth receive / scan**: decrypt an inbound
/// stealth UTXO with a view secret and decide whether it is addressed to the scanner. Its `input`
/// carries a [`StealthScanInput`] (`stealth_scan_input`).
///
/// **This vector uses the default `"bytes"` comparison mode.** Decryption is the deterministic
/// inverse of encryption — it calls **no** RNG — so the produced [`DecryptedOutput`] (or `null` for
/// a not-mine UTXO) is fully byte-stable and compared directly. The runner serializes the produced
/// `Option<DecryptedOutput>` and asserts it equals `expected.decrypted`.
pub const OP_SCAN_STEALTH_OUTPUT: &str = "scan_stealth_output";

/// The canonical operation id for the **stealth UTXO decode**: turn a fetched UTXO substate (id +
/// value) into the receive-shaped [`ootle_sdk_core::types::stealth::InboundStealthOutput`]. Its
/// `input` carries `substate_id` + `substate_value` (the JSON the indexer returned, verbatim);
/// `expected.inbound_output` carries the decoded `InboundStealthOutput`.
///
/// **Default `"bytes"` comparison mode.** The decode is a pure parse + field map (no RNG), so the
/// produced `InboundStealthOutput` is byte-stable and compared as a JSON object directly.
pub const OP_DECODE_STEALTH_UTXO: &str = "decode_stealth_utxo";

/// The canonical operation id for **deterministic account keygen**: derive an account keypair from a
/// 32-byte seed. Its `input.seed` carries the lowercase-hex seed; `expected.keypair` carries the
/// `{account_secret, account_public_key}` record.
///
/// **Default `"bytes"` comparison mode.** The seed path calls **no** RNG (it derives the scalar via the
/// canonical KDF), so the produced keypair is byte-stable and compared as a JSON object directly.
pub const OP_DERIVE_ACCOUNT_KEY_FROM_SEED: &str = "derive_account_key_from_seed";

/// The canonical operation id for **deterministic view keygen**: derive a view keypair from a 32-byte
/// seed. Its `input.seed` carries the lowercase-hex seed; `expected.keypair` carries the
/// `{view_secret, view_public_key}` record. Byte-stable (`"bytes"`).
pub const OP_DERIVE_VIEW_KEY_FROM_SEED: &str = "derive_view_key_from_seed";

/// The canonical operation id for **account-address derivation**: hash an account public key into its
/// canonical `component_<hex>` address via the engine derivation. Its `input.account_public_key` carries
/// the lowercase-hex 32-byte public key; `expected.component_address` carries the derived address.
///
/// **Default `"bytes"` comparison mode.** The derivation is a domain-separated Blake2b hash with no RNG,
/// so the produced `component_<hex>` is byte-stable. This is the **lost-funds vector** — a wrong hash
/// would place funds at an address nobody controls — so it is locked byte-for-byte and cross-checked
/// against the live transfer-builder's recipient derivation (see the generator).
pub const OP_DERIVE_ACCOUNT_ADDRESS: &str = "derive_account_address";

/// The canonical operation id for **identity-address formatting**: encode an
/// `{network, account_key, view_only_key, pay_ref?}` record into its canonical `otl_…` bech32m
/// string. Its `input` carries `network` + `account_key` + `view_only_key` (lowercase hex) + an
/// optional `pay_ref` (lowercase hex); `expected.bech32m` carries the encoded string.
///
/// **Default `"bytes"` comparison mode.** bech32m encoding is RNG-free and deterministic, so the
/// produced string is byte-stable and compared exactly. The HRP is network-qualified, so the
/// multi-network vectors lock that the encoder picks the right HRP per network.
pub const OP_FORMAT_IDENTITY_ADDRESS: &str = "format_identity_address";

/// The canonical operation id for **address parsing**: parse a `component_/resource_<hex>` substate
/// id **or** an `otl_…` bech32m identity address into the kind-tagged [`ootle_sdk_core::ParsedAddress`].
/// Its `input.address` carries the string; `expected.parsed_address` carries the serialized
/// `ParsedAddress` (kind-tagged JSON).
///
/// **Default `"bytes"` comparison mode** on a structured value: parsing is RNG-free, so the produced
/// `ParsedAddress` is byte-stable and compared as a JSON object. The identity-parse vector's
/// `input.address` is a string produced by `format_identity_address` (the same crate codec a wallet
/// uses), so a parse vector cross-checks a wallet-shaped `otl_…` string round-trips its fields.
pub const OP_PARSE_ADDRESS: &str = "parse_address";

/// The canonical operation id for **typed substate decode**: turn a fetched [`SubstateValue`] JSON
/// into the kind-tagged [`ootle_sdk_core::DecodedSubstate`]. Its `input.substate_value` carries the
/// indexer's substate JSON verbatim; `expected.decoded_substate` carries the decoded record.
///
/// **Default `"bytes"` comparison mode.** The decode is a pure parse + field map (no RNG), so the
/// produced record is byte-stable; the embedded `u64` balances are native JSON numbers.
pub const OP_DECODE_SUBSTATE: &str = "decode_substate";

/// The canonical operation id for **account balances**: an account component substate + its already-
/// fetched vault substates → the revealed balance per resource. Its `input.substate_value` carries the
/// account component JSON and `input.vault_substates` carries the vault `FetchedSubstate`s;
/// `expected.account_balances` carries the `Vec<ResourceBalance>`.
///
/// **Default `"bytes"` comparison mode.** The sum is RNG-free and the `u64` balances are native JSON
/// numbers (a `> 2^33` balance is locked here).
pub const OP_ACCOUNT_BALANCES: &str = "account_balances";

/// The canonical operation id for **typed arg encoding**: encode one [`ArgValue`] onto the builder's
/// own `InstructionArg` / `tari_bor` seam (`encode_arg`). Its `input.arg_value` carries the
/// [`ArgValue`]; `expected.encoded_arg_bytes` carries the **literal CBOR bytes** as lowercase hex.
///
/// **Default `"bytes"` comparison mode.** Arg encoding is RNG-free and must be byte-identical to the
/// engine's wire format, so the literal bytes are locked byte-for-byte. A host that re-ports the
/// literal encoder and gets a single byte wrong fails this vector — the lost-funds drift class this
/// fixture group exists to prevent.
pub const OP_ENCODE_ARG: &str = "encode_arg";

/// The canonical operation id for the **co-sign seal** (party A): build the resolved partial from
/// `network`/`intent`/`fetched`, attach the supplied authorizations, and seal deterministically with
/// A's pinned `keys`. Its `input` carries `network`/`intent`/`fetched`/`keys` + the cosign authorize
/// fields (the harness re-derives B's authorization the same way the `cosign_add_signature` op does);
/// `expected.sealed_transaction_semantic` carries the decoded sealed tx with byte-unstable fields nulled.
///
/// **`"semantic"` comparison mode.** Matches the stealth-send precedent: the runner re-seals, verifies
/// every signature via [`ootle_sdk_core::decode_and_canonicalize_sealed_transfer`], and compares the
/// deterministic decoded fields (the signer public keys + `main_signer` survive, locking
/// the seal-signer-authorized contract; the Schnorr scalars are nulled).
pub const OP_COSIGN_SEAL_WITH_AUTH: &str = "cosign_seal_with_auth";

/// The canonical operation id for the **generic instruction front-end**: lower a
/// [`ootle_sdk_core::GenericTransactionIntent`] onto the two-phase pipeline, apply one fetched batch,
/// and seal/encode deterministically (`resolve_and_encode_instructions`). Its `input` carries a
/// `generic_intent` + (optional) `fetched` + the deterministic `keys`.
///
/// **Default `"bytes"` comparison mode.** The deterministic public-transfer seal path is byte-stable
/// (the precedent: `resolve_and_encode_public_transfer` vectors are `"bytes"`), so the
/// `instructions → encoded transaction` bytes + id are locked byte-for-byte. The point is to prove the
/// generic front-end produces the SAME submit-ready bytes as the bespoke builder for an equivalent
/// intent.
pub const OP_BUILD_AND_ENCODE_INSTRUCTIONS: &str = "build_and_encode_instructions";

/// The canonical operation id for the **first-class faucet claim builder**: build the self-funding
/// claim, apply one fetched batch (the faucet component + its vault), and seal/encode deterministically.
/// Its `input` carries a `faucet_intent` + the `fetched` faucet substates + the deterministic `keys`.
/// `"bytes"` comparison mode, like the generic builder.
pub const OP_BUILD_AND_ENCODE_FAUCET_CLAIM: &str = "build_and_encode_faucet_claim";

/// One golden-vector fixture file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fixture {
    /// Stable id, e.g. `public_transfer/single_key_basic`. Surfaced in failure messages.
    pub name: String,
    /// Fixture-format schema version (bump on format changes).
    pub schema_version: u32,
    /// How/when the fixture was produced (traceability, not silent magic).
    pub provenance: Provenance,
    /// The core entry point the fixture exercises.
    pub operation: String,
    /// The comparison mode for this fixture. `"bytes"` (the default for the encode/parse ops) means
    /// the runner asserts byte-for-byte; `"semantic"` (the stealth ops) means it validates
    /// cryptographically + compares deterministic fields. Existing fixtures omit it
    /// (`#[serde(default)]` → `"bytes"`).
    #[serde(default = "default_compare", skip_serializing_if = "is_bytes_compare")]
    pub compare: String,
    /// The fully-deterministic arguments to the core function (no RNG, no clock).
    pub input: VectorInput,
    /// The byte-exact outputs (generator-owned, lowercase hex).
    pub expected: ExpectedOutput,
}

/// The default comparison mode (`"bytes"`) for fixtures that omit `compare`.
pub fn default_compare() -> String {
    "bytes".to_string()
}

/// Whether `compare` is the default (`"bytes"`) — used to keep existing fixtures byte-identical
/// (they don't serialize a `compare` field).
fn is_bytes_compare(s: &str) -> bool {
    s == "bytes"
}

/// Records what produced a fixture so a regenerated fixture is traceable.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provenance {
    /// The `ootle_sdk_core` crate version that generated the fixture.
    pub core_version: String,
    /// The git revision (`git rev-parse HEAD`) at generation time, or `"unknown"`.
    pub git_rev: String,
    /// What generated it, e.g. `ootle_sdk_core golden-vector generator`.
    pub generated_by: String,
}

/// The arguments to [`build_and_encode_public_transfer`], serialized language-neutrally.
///
/// This is exactly the operation's `input`: the boundary [`Network`], the [`PublicTransferIntent`]
/// (which already serializes via serde with hex bytes), and the **deterministic** key bundle with its
/// pinned nonce secrets. Every byte newtype here serializes as lowercase hex.
///
/// For the **parse** op the encode fields (`network`/`intent`/`keys`) are absent and `raw_result`
/// carries the raw indexer JSON instead. All op-specific fields are `#[serde(default, skip_if_none)]`
/// so each fixture only serializes the fields its op uses, and existing fixtures deserialize unchanged.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VectorInput {
    /// The boundary network (serialized as a lowercase keyword, e.g. `"esmeralda"`). Encode ops only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network: Option<Network>,
    /// The public-transfer intent. Encode ops only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent: Option<PublicTransferIntent>,
    /// The pinned, deterministic key + nonce material. Encode ops only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keys: Option<VectorKeys>,
    /// The fetched substates feeding the **resolved** path. Present only for
    /// `resolve_and_encode_public_transfer` vectors; `#[serde(default)]` keeps the existing
    /// `build_and_encode_public_transfer` fixtures (which omit it) deserializing unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fetched: Option<Vec<FetchedSubstate>>,
    /// The raw indexer finalized-result JSON, committed verbatim. Present only for
    /// `parse_finalized_result` vectors (the `parse` op input).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_result: Option<serde_json::Value>,
    /// The stealth transfer intent. Present only for `build_stealth_outputs_statement` vectors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stealth_intent: Option<ootle_sdk_core::types::stealth::StealthTransferIntent>,
    /// The build seed expanded into the stealth proof entropy. Present only for the
    /// `build_stealth_outputs_statement` vectors (the full-send vectors carry their seed in
    /// `stealth_keys.seed`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stealth_seed: Option<BuildSeed>,
    /// The pinned stealth seal-key bundle (carries the build seed). Present only for
    /// `build_and_encode_stealth_transfer` vectors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stealth_keys: Option<VectorStealthKeys>,
    /// The per-input view-only spend secrets (positional, one per `stealth_intent.inputs`) that
    /// decrypt the stealth-input masks. Present only for `build_and_encode_stealth_transfer` vectors
    /// with stealth inputs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub spend_secrets: Vec<SecretKeyBytes>,
    /// The stealth receive/scan arguments. Present only for `scan_stealth_output` vectors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stealth_scan_input: Option<StealthScanInput>,
    /// The lowercase-hex 32-byte seed feeding the deterministic keygen ops. Present only for
    /// `derive_account_key_from_seed` / `derive_view_key_from_seed` vectors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<String>,
    /// The lowercase-hex 32-byte account public key feeding `derive_account_address`. Present only for
    /// `derive_account_address` vectors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_public_key: Option<String>,
    /// The lowercase-hex 32-byte view-only public key feeding `format_identity_address`. Present only
    /// for `format_identity_address` vectors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view_only_key: Option<String>,
    /// The optional lowercase-hex payment reference feeding `format_identity_address`. Present (and
    /// non-null) only for the `format_identity_address` vectors that carry a pay_ref.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pay_ref: Option<String>,
    /// The address string feeding `parse_address` (a `component_/resource_<hex>` id or an `otl_…`
    /// identity). Present only for `parse_address` vectors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    /// The fetched UTXO substate id (`utxo_<resource>_<commitment>`) feeding `decode_stealth_utxo`.
    /// Present only for `decode_stealth_utxo` vectors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub substate_id: Option<String>,
    /// The fetched `SubstateValue` JSON, committed verbatim (the indexer's shape). Feeds
    /// `decode_stealth_utxo` (the UTXO), `decode_substate` (any substate), and `account_balances` (the
    /// account component).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub substate_value: Option<serde_json::Value>,
    /// The account's already-fetched vault substates (id + value), feeding `account_balances`. Present
    /// only for `account_balances` vectors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vault_substates: Option<Vec<FetchedSubstate>>,
    /// The typed argument feeding `encode_arg`. Present only for `encode_arg` vectors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arg_value: Option<ArgValue>,
    /// The generic transaction intent feeding `build_and_encode_instructions`. Present only for
    /// `build_and_encode_instructions` vectors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generic_intent: Option<ootle_sdk_core::GenericTransactionIntent>,
    /// The faucet claim intent feeding `build_and_encode_faucet_claim`. Present only for
    /// `build_and_encode_faucet_claim` vectors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub faucet_intent: Option<ootle_sdk_core::FaucetClaimIntent>,
    /// Party A's seal public key (lowercase hex), feeding the co-sign ops. Present only for
    /// `cosign_add_signature` / `cosign_seal_with_auth` vectors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cosign_seal_pk: Option<String>,
    /// Party B's co-signer secret key, feeding the co-sign ops. Present only for the co-sign vectors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cosign_signer_secret: Option<SecretKeyBytes>,
    /// Party B's build seed, expanded into the cosign authorization nonce (seed-reproducible co-sign).
    /// Present only for the co-sign vectors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cosign_signer_seed: Option<BuildSeed>,
}

/// The arguments to [`ootle_sdk_core::scan_stealth_output`], serialized language-neutrally
/// (`scan_stealth_output` op input).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StealthScanInput {
    /// The network whose hash domains parameterize the tag / spend-key derivation.
    pub network: Network,
    /// The recipient's view-only secret (re-derives the AEAD key with the sender's public nonce).
    pub view_secret: SecretKeyBytes,
    /// The recipient's account secret, for the ownership (tag + spend-key) checks. `None` selects
    /// view-key-only mode (those checks are skipped).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_secret: Option<SecretKeyBytes>,
    /// The inbound stealth UTXO to scan.
    pub output: ootle_sdk_core::types::stealth::InboundStealthOutput,
    /// When `true`, the memo region is not decoded (the result's `memo` is `None`).
    pub skip_memo: bool,
}

/// A serde-friendly mirror of [`StealthKeys`] (the core type carries secret material and does not
/// derive `Serialize`). [`VectorStealthKeys::to_core`] reconstitutes the core bundle. The `seed` here
/// is the **construction** seed (masks/AEAD/proof entropy) the stealth build consumes — the signatures
/// are random.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorStealthKeys {
    /// The account secret key.
    pub account_secret: SecretKeyBytes,
    /// The build seed expanded into the per-output construction entropy (commitments/AEAD/proofs).
    pub seed: BuildSeed,
}

impl VectorStealthKeys {
    /// Reconstitutes the core stealth seal-key bundle.
    pub fn to_core(&self) -> StealthKeys {
        StealthKeys::new(self.account_secret.clone())
    }
}

/// A serde-friendly mirror of [`PublicTransferKeys`] (the core type intentionally does **not** derive
/// `Serialize`, since it carries secret material). The harness owns the language-neutral wire shape;
/// [`VectorKeys::to_core`] reconstitutes the core type the operation consumes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorKeys {
    /// The account secret key (authorization signer + seal signer for the single-key path).
    pub account_secret: SecretKeyBytes,
}

impl VectorKeys {
    /// Reconstitutes the core key bundle the operation consumes.
    pub fn to_core(&self) -> PublicTransferKeys {
        PublicTransferKeys::new(self.account_secret.clone())
    }
}

/// The generator-owned outputs of an operation.
///
/// Encode ops fill the two **byte-exact** hex fields (`encoded_transaction`/`transaction_id`); the
/// **parse** op fills `parsed` with the canonical JSON of the produced `FinalizedResult` instead (a
/// structural compare, not a byte compare — see [`OP_PARSE_FINALIZED_RESULT`]). Each op serializes only
/// the fields it uses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ExpectedOutput {
    /// The submit-ready BOR-encoded transaction bytes — **canonical encoding: lowercase hex**. Encode ops.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub encoded_transaction: String,
    /// The transaction id (32 bytes), lowercase hex. Encode ops.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub transaction_id: String,
    /// The parsed `FinalizedResult` as canonical JSON. Parse op only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parsed: Option<serde_json::Value>,
    /// The built `StealthOutputsStatement` as canonical JSON with `agg_range_proof` nulled out.
    /// `build_stealth_outputs_statement` (semantic) op only — see [`OP_BUILD_STEALTH_OUTPUTS_STATEMENT`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stealth_outputs_statement: Option<serde_json::Value>,
    /// The aggregated output mask (32-byte hex) the statement build returns alongside the statement.
    /// Deterministic, so byte-compared even in `"semantic"` mode. Stealth op only.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub aggregated_output_mask: String,
    /// The **decoded** sealed transaction as canonical JSON, with the byte-unstable fields (the
    /// aggregated bulletproof, the balance-proof signature, and every Schnorr signature scalar) nulled
    /// out. `build_and_encode_stealth_transfer` (semantic) op only — see
    /// [`OP_BUILD_AND_ENCODE_STEALTH_TRANSFER`]. The seal/authorization signer **public keys** survive
    /// (they are deterministic), so this still locks the key-selection contract.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sealed_transaction_semantic: Option<serde_json::Value>,
    /// The scan result for `scan_stealth_output`: the recovered [`DecryptedOutput`] as
    /// canonical JSON, or JSON `null` when the UTXO is not addressed to the scanner. **Byte-stable**
    /// (decryption is RNG-free), so compared directly. Distinguished from an *absent* field — a
    /// not-mine scan records `"decrypted": null`, which `Option::is_none` would wrongly skip, so this
    /// field is always serialized for the scan op (see [`OP_SCAN_STEALTH_OUTPUT`]).
    #[serde(default, skip_serializing_if = "is_unset_value")]
    pub decrypted: serde_json::Value,
    /// The derived keypair for the `derive_*_key_from_seed` ops: a JSON object
    /// `{"account_secret","account_public_key"}` (account) or `{"view_secret","view_public_key"}`
    /// (view), lowercase hex. **Byte-stable** (the seed path is RNG-free), so compared as a JSON object.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keypair: Option<serde_json::Value>,
    /// The derived account address (canonical `component_<hex>`) for the `derive_account_address` op.
    /// **Byte-stable** (the derivation is an RNG-free domain-separated hash), so compared as a string.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component_address: Option<String>,
    /// The encoded `otl_…` bech32m string for the `format_identity_address` op. **Byte-stable**
    /// (bech32m is RNG-free), so compared as a string.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bech32m: Option<String>,
    /// The serialized kind-tagged [`ootle_sdk_core::ParsedAddress`] for the `parse_address` op.
    /// **Byte-stable** (parsing is RNG-free), so compared as a JSON object.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parsed_address: Option<serde_json::Value>,
    /// The decoded [`ootle_sdk_core::types::stealth::InboundStealthOutput`] for the
    /// `decode_stealth_utxo` op. **Byte-stable** (the decode is RNG-free), so compared as a JSON
    /// object.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inbound_output: Option<serde_json::Value>,
    /// The decoded kind-tagged [`ootle_sdk_core::DecodedSubstate`] for the `decode_substate` op.
    /// **Byte-stable** (the decode is RNG-free); `u64` balances are native JSON numbers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decoded_substate: Option<serde_json::Value>,
    /// The `Vec<ResourceBalance>` for the `account_balances` op. **Byte-stable** (the sum is RNG-free);
    /// `u64` revealed balances are native JSON numbers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_balances: Option<serde_json::Value>,
    /// The encoded literal CBOR bytes (lowercase hex) of the `encode_arg` op's [`ArgValue`].
    /// **Byte-for-byte** (see [`OP_ENCODE_ARG`]).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub encoded_arg_bytes: String,
    /// The co-signer's [`ootle_sdk_core::Authorization`] (`{public_key, signature}`) as canonical JSON
    /// for the `cosign_add_signature` op. **Byte-stable** (pinned-nonce, RNG-free), compared directly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cosign_authorization: Option<serde_json::Value>,
}

/// Whether a `serde_json::Value` is the harness's "field unused by this op" sentinel.
///
/// `ExpectedOutput::default()` leaves `decrypted` as `Value::Null` (serde's `Default`). A *scan*
/// fixture, however, legitimately records `"decrypted": null` for a not-mine UTXO, so we cannot key
/// off `null` to decide whether to serialize. The scan op always sets `decrypted` to a JSON object
/// (mine) or an explicit `Value::Null` wrapped so it is never the default sentinel — see
/// [`run_operation`], which uses [`scan_value`] to record the result. The sentinel for "no scan op"
/// is the absence of any assignment, which serde renders as `Value::Null`; to keep non-scan fixtures
/// byte-identical we suppress that. The scan op records its `null` via a tagged wrapper that is not
/// `Value::Null`, so it survives.
fn is_unset_value(v: &serde_json::Value) -> bool {
    v.is_null()
}

/// Records a scan result as a serde value that is **never** the unused-field sentinel: a mine result
/// is its canonical-JSON object; a not-mine result is the string-free JSON `null` re-tagged through
/// an object wrapper `{ "$none": true }` so it serializes (and a runner can distinguish it).
fn scan_value(result: &Option<ootle_sdk_core::types::stealth::DecryptedOutput>) -> serde_json::Value {
    match result {
        Some(d) => canonicalize_json(serde_json::to_value(d).expect("DecryptedOutput serializes")),
        None => serde_json::json!({ "$none": true }),
    }
}

/// Runs the operation named by a fixture over its `input` and returns the byte-exact output as hex.
///
/// This is the single place that maps `operation` → core call, used by **both** the generator (to
/// fill `expected`) and the runner (to check it). Keeping them on one code path guarantees the
/// generator and runner can never silently diverge.
#[allow(clippy::too_many_lines)] // one match arm per fixture operation; see doc above
pub fn run_operation(fixture: &Fixture) -> ExpectedOutput {
    match fixture.operation.as_str() {
        OP_BUILD_AND_ENCODE_PUBLIC_TRANSFER => {
            let input = &fixture.input;
            let network = input
                .network
                .unwrap_or_else(|| panic!("fixture `{}`: encode op requires `input.network`", fixture.name));
            let intent = input
                .intent
                .as_ref()
                .unwrap_or_else(|| panic!("fixture `{}`: encode op requires `input.intent`", fixture.name));
            let keys = input
                .keys
                .as_ref()
                .unwrap_or_else(|| panic!("fixture `{}`: encode op requires `input.keys`", fixture.name));
            let out = build_and_encode_public_transfer(network, intent, &keys.to_core())
                .unwrap_or_else(|e| panic!("fixture `{}`: core operation failed: {e}", fixture.name));
            ExpectedOutput {
                sealed_transaction_semantic: Some(decoded_semantic_transaction(&fixture.name, &out)),
                ..Default::default()
            }
        },
        OP_RESOLVE_AND_ENCODE_PUBLIC_TRANSFER => {
            let input = &fixture.input;
            let network = input
                .network
                .unwrap_or_else(|| panic!("fixture `{}`: encode op requires `input.network`", fixture.name));
            let intent = input
                .intent
                .as_ref()
                .unwrap_or_else(|| panic!("fixture `{}`: encode op requires `input.intent`", fixture.name));
            let keys = input
                .keys
                .as_ref()
                .unwrap_or_else(|| panic!("fixture `{}`: encode op requires `input.keys`", fixture.name));
            let fetched = input.fetched.as_deref().unwrap_or_else(|| {
                panic!(
                    "fixture `{}`: resolve_and_encode requires `input.fetched`",
                    fixture.name
                )
            });
            let (partial, _wants) = build_public_transfer_unsigned_with_wants(network, intent)
                .unwrap_or_else(|e| panic!("fixture `{}`: build-with-wants failed: {e}", fixture.name));
            let resolved = match apply_fetched_substates(partial, fetched)
                .unwrap_or_else(|e| panic!("fixture `{}`: apply failed: {e}", fixture.name))
            {
                Resolution::Resolved(p) => p,
                Resolution::NeedMore { .. } => {
                    panic!("fixture `{}`: did not resolve from the fetched batch", fixture.name)
                },
            };
            let out = seal_and_encode_public_transfer(resolved, &keys.to_core())
                .unwrap_or_else(|e| panic!("fixture `{}`: seal failed: {e}", fixture.name));
            ExpectedOutput {
                sealed_transaction_semantic: Some(decoded_semantic_transaction(&fixture.name, &out)),
                ..Default::default()
            }
        },
        OP_BUILD_AND_ENCODE_INSTRUCTIONS => {
            let input = &fixture.input;
            let network = input
                .network
                .unwrap_or_else(|| panic!("fixture `{}`: encode op requires `input.network`", fixture.name));
            let intent = input.generic_intent.as_ref().unwrap_or_else(|| {
                panic!(
                    "fixture `{}`: build_and_encode_instructions requires `input.generic_intent`",
                    fixture.name
                )
            });
            let keys = input
                .keys
                .as_ref()
                .unwrap_or_else(|| panic!("fixture `{}`: encode op requires `input.keys`", fixture.name));
            let fetched = input.fetched.as_deref().unwrap_or(&[]);
            let (partial, _wants) = build_unsigned_instructions_with_wants(network, intent)
                .unwrap_or_else(|e| panic!("fixture `{}`: build-with-wants failed: {e}", fixture.name));
            let resolved = match apply_fetched_substates(partial, fetched)
                .unwrap_or_else(|e| panic!("fixture `{}`: apply failed: {e}", fixture.name))
            {
                Resolution::Resolved(p) => p,
                Resolution::NeedMore { .. } => {
                    panic!("fixture `{}`: did not resolve from the fetched batch", fixture.name)
                },
            };
            let out = seal_and_encode_public_transfer(resolved, &keys.to_core())
                .unwrap_or_else(|e| panic!("fixture `{}`: seal failed: {e}", fixture.name));
            ExpectedOutput {
                sealed_transaction_semantic: Some(decoded_semantic_transaction(&fixture.name, &out)),
                ..Default::default()
            }
        },
        OP_BUILD_AND_ENCODE_FAUCET_CLAIM => {
            let input = &fixture.input;
            let network = input
                .network
                .unwrap_or_else(|| panic!("fixture `{}`: encode op requires `input.network`", fixture.name));
            let intent = input.faucet_intent.as_ref().unwrap_or_else(|| {
                panic!(
                    "fixture `{}`: build_and_encode_faucet_claim requires `input.faucet_intent`",
                    fixture.name
                )
            });
            let keys = input
                .keys
                .as_ref()
                .unwrap_or_else(|| panic!("fixture `{}`: encode op requires `input.keys`", fixture.name));
            let fetched = input.fetched.as_deref().unwrap_or(&[]);
            let (partial, _wants) = build_faucet_claim_with_wants(network, intent)
                .unwrap_or_else(|e| panic!("fixture `{}`: faucet build failed: {e}", fixture.name));
            let resolved = match apply_fetched_substates(partial, fetched)
                .unwrap_or_else(|e| panic!("fixture `{}`: apply failed: {e}", fixture.name))
            {
                Resolution::Resolved(p) => p,
                Resolution::NeedMore { .. } => panic!(
                    "fixture `{}`: faucet claim did not resolve from the fetched batch (provide the faucet component \
                     + its vault)",
                    fixture.name
                ),
            };
            let out = seal_and_encode_public_transfer(resolved, &keys.to_core())
                .unwrap_or_else(|e| panic!("fixture `{}`: seal failed: {e}", fixture.name));
            ExpectedOutput {
                sealed_transaction_semantic: Some(decoded_semantic_transaction(&fixture.name, &out)),
                ..Default::default()
            }
        },
        OP_PARSE_FINALIZED_RESULT => {
            let raw = fixture.input.raw_result.as_ref().unwrap_or_else(|| {
                panic!(
                    "fixture `{}`: parse_finalized_result requires `input.raw_result`",
                    fixture.name
                )
            });
            // The host hands the core the raw JSON *string*; mirror that exactly (serialize the
            // committed `Value` back to its string form, then parse it).
            let raw_str = serde_json::to_string(raw).expect("raw_result re-serializes");
            let parsed = parse_finalized_result(&raw_str)
                .unwrap_or_else(|e| panic!("fixture `{}`: parse failed: {e}", fixture.name));
            // Canonicalize the parsed record so the compare is order-insensitive.
            let value = serde_json::to_value(&parsed).expect("FinalizedResult serializes");
            ExpectedOutput {
                parsed: Some(canonicalize_json(value)),
                ..Default::default()
            }
        },
        OP_BUILD_STEALTH_OUTPUTS_STATEMENT => {
            let input = &fixture.input;
            let network = input
                .network
                .unwrap_or_else(|| panic!("fixture `{}`: stealth op requires `input.network`", fixture.name));
            let intent = input
                .stealth_intent
                .as_ref()
                .unwrap_or_else(|| panic!("fixture `{}`: stealth op requires `input.stealth_intent`", fixture.name));
            let seed = input
                .stealth_seed
                .unwrap_or_else(|| panic!("fixture `{}`: stealth op requires `input.stealth_seed`", fixture.name));
            let (stmt, mask) =
                ootle_sdk_core::stealth::build_stealth_outputs_statement_with_seed(network, intent, &seed, None)
                    .unwrap_or_else(|e| panic!("fixture `{}`: core operation failed: {e}", fixture.name));
            // Record the deterministic fields: serialize the statement, then null out the
            // (byte-unstable) aggregated range proof so the semantic compare is stable.
            let mut value = serde_json::to_value(&stmt).expect("StealthOutputsStatement serializes");
            if let Some(obj) = value.as_object_mut() {
                obj.insert("agg_range_proof".to_string(), serde_json::Value::Null);
            }
            ExpectedOutput {
                stealth_outputs_statement: Some(canonicalize_json(value)),
                aggregated_output_mask: mask.to_hex(),
                ..Default::default()
            }
        },
        OP_BUILD_AND_ENCODE_STEALTH_TRANSFER => {
            let input = &fixture.input;
            let network = input
                .network
                .unwrap_or_else(|| panic!("fixture `{}`: stealth send requires `input.network`", fixture.name));
            let intent = input.stealth_intent.as_ref().unwrap_or_else(|| {
                panic!(
                    "fixture `{}`: stealth send requires `input.stealth_intent`",
                    fixture.name
                )
            });
            let keys = input
                .stealth_keys
                .as_ref()
                .unwrap_or_else(|| panic!("fixture `{}`: stealth send requires `input.stealth_keys`", fixture.name));
            let fetched = input.fetched.as_deref().unwrap_or(&[]);
            // The keys bundle's build seed drives both the account-key nonces and the proof entropy.
            let seed = keys.seed;
            let out = build_and_encode_stealth_transfer_with_seed(
                network,
                intent,
                fetched,
                &input.spend_secrets,
                &keys.to_core(),
                &seed,
            )
            .unwrap_or_else(|e| panic!("fixture `{}`: stealth send failed: {e}", fixture.name));
            ExpectedOutput {
                sealed_transaction_semantic: Some(decoded_semantic_transaction(&fixture.name, &out)),
                ..Default::default()
            }
        },
        OP_SCAN_STEALTH_OUTPUT => {
            let scan = fixture.input.stealth_scan_input.as_ref().unwrap_or_else(|| {
                panic!(
                    "fixture `{}`: scan op requires `input.stealth_scan_input`",
                    fixture.name
                )
            });
            let result = ootle_sdk_core::scan_stealth_output(
                scan.network,
                &scan.view_secret,
                scan.account_secret.as_ref(),
                &scan.output,
                scan.skip_memo,
            )
            .unwrap_or_else(|e| panic!("fixture `{}`: scan failed: {e}", fixture.name));
            ExpectedOutput {
                decrypted: scan_value(&result),
                ..Default::default()
            }
        },
        OP_DERIVE_ACCOUNT_KEY_FROM_SEED => {
            let seed = seed_from_fixture(fixture);
            let kp = derive_account_keypair_from_seed(&seed)
                .unwrap_or_else(|e| panic!("fixture `{}`: account keygen failed: {e}", fixture.name));
            ExpectedOutput {
                keypair: Some(serde_json::json!({
                    "account_secret": kp.secret.to_hex(),
                    "account_public_key": kp.public_key.to_hex(),
                })),
                ..Default::default()
            }
        },
        OP_DERIVE_VIEW_KEY_FROM_SEED => {
            let seed = seed_from_fixture(fixture);
            let kp = derive_view_keypair_from_seed(&seed)
                .unwrap_or_else(|e| panic!("fixture `{}`: view keygen failed: {e}", fixture.name));
            ExpectedOutput {
                keypair: Some(serde_json::json!({
                    "view_secret": kp.secret.to_hex(),
                    "view_public_key": kp.public_key.to_hex(),
                })),
                ..Default::default()
            }
        },
        OP_DERIVE_ACCOUNT_ADDRESS => {
            let pk_hex = fixture.input.account_public_key.as_ref().unwrap_or_else(|| {
                panic!(
                    "fixture `{}`: derive_account_address requires `input.account_public_key`",
                    fixture.name
                )
            });
            let pk = PublicKeyBytes::from_hex(pk_hex)
                .unwrap_or_else(|e| panic!("fixture `{}`: bad account_public_key hex: {e}", fixture.name));
            let component = derive_account_address(&pk)
                .unwrap_or_else(|e| panic!("fixture `{}`: address derivation failed: {e}", fixture.name));
            ExpectedOutput {
                component_address: Some(component.as_str().to_string()),
                ..Default::default()
            }
        },
        OP_FORMAT_IDENTITY_ADDRESS => {
            let input = &fixture.input;
            let network = input.network.unwrap_or_else(|| {
                panic!(
                    "fixture `{}`: format_identity_address requires `input.network`",
                    fixture.name
                )
            });
            let account_pk = input.account_public_key.as_ref().unwrap_or_else(|| {
                panic!(
                    "fixture `{}`: format_identity_address requires `input.account_public_key`",
                    fixture.name
                )
            });
            let view_pk = input.view_only_key.as_ref().unwrap_or_else(|| {
                panic!(
                    "fixture `{}`: format_identity_address requires `input.view_only_key`",
                    fixture.name
                )
            });
            let account = PublicKeyBytes::from_hex(account_pk)
                .unwrap_or_else(|e| panic!("fixture `{}`: bad account_public_key hex: {e}", fixture.name));
            let view = PublicKeyBytes::from_hex(view_pk)
                .unwrap_or_else(|e| panic!("fixture `{}`: bad view_only_key hex: {e}", fixture.name));
            let bech32m = format_identity_address(network, &account, &view, input.pay_ref.as_deref())
                .unwrap_or_else(|e| panic!("fixture `{}`: format_identity_address failed: {e}", fixture.name));
            ExpectedOutput {
                bech32m: Some(bech32m),
                ..Default::default()
            }
        },
        OP_PARSE_ADDRESS => {
            let address = fixture
                .input
                .address
                .as_ref()
                .unwrap_or_else(|| panic!("fixture `{}`: parse_address requires `input.address`", fixture.name));
            let parsed = parse_address(address)
                .unwrap_or_else(|e| panic!("fixture `{}`: parse_address failed: {e}", fixture.name));
            let value = serde_json::to_value(&parsed).expect("ParsedAddress serializes");
            ExpectedOutput {
                parsed_address: Some(canonicalize_json(value)),
                ..Default::default()
            }
        },
        OP_DECODE_STEALTH_UTXO => {
            let substate_id = fixture.input.substate_id.as_ref().unwrap_or_else(|| {
                panic!(
                    "fixture `{}`: decode_stealth_utxo requires `input.substate_id`",
                    fixture.name
                )
            });
            let substate_value = fixture.input.substate_value.as_ref().unwrap_or_else(|| {
                panic!(
                    "fixture `{}`: decode_stealth_utxo requires `input.substate_value`",
                    fixture.name
                )
            });
            // The host hands the core the indexer's SubstateValue JSON verbatim; mirror that exactly.
            let inbound = decode_stealth_utxo(substate_id, substate_value)
                .unwrap_or_else(|e| panic!("fixture `{}`: decode_stealth_utxo failed: {e}", fixture.name));
            let json = serde_json::to_value(&inbound).expect("InboundStealthOutput serializes");
            ExpectedOutput {
                inbound_output: Some(canonicalize_json(json)),
                ..Default::default()
            }
        },
        OP_DECODE_SUBSTATE => {
            let substate_value = fixture.input.substate_value.as_ref().unwrap_or_else(|| {
                panic!(
                    "fixture `{}`: decode_substate requires `input.substate_value`",
                    fixture.name
                )
            });
            let decoded = decode_substate(substate_value)
                .unwrap_or_else(|e| panic!("fixture `{}`: decode_substate failed: {e}", fixture.name));
            let json = serde_json::to_value(&decoded).expect("DecodedSubstate serializes");
            ExpectedOutput {
                decoded_substate: Some(canonicalize_json(json)),
                ..Default::default()
            }
        },
        OP_ACCOUNT_BALANCES => {
            let account = fixture.input.substate_value.as_ref().unwrap_or_else(|| {
                panic!(
                    "fixture `{}`: account_balances requires `input.substate_value` (the account component)",
                    fixture.name
                )
            });
            let vaults = fixture.input.vault_substates.as_deref().unwrap_or_else(|| {
                panic!(
                    "fixture `{}`: account_balances requires `input.vault_substates`",
                    fixture.name
                )
            });
            let balances = account_balances(account, vaults)
                .unwrap_or_else(|e| panic!("fixture `{}`: account_balances failed: {e}", fixture.name));
            let json = serde_json::to_value(&balances).expect("Vec<ResourceBalance> serializes");
            ExpectedOutput {
                account_balances: Some(canonicalize_json(json)),
                ..Default::default()
            }
        },
        OP_ENCODE_ARG => {
            let arg = fixture.input.arg_value.as_ref().unwrap_or_else(|| {
                panic!("fixture `{}`: encode_arg requires `input.arg_value`", fixture.name);
            });
            // Workspace args are intentionally not standalone-encodable (their numeric id is
            // builder-stateful). Reject them up front with a clear message rather than surfacing the
            // generic "encode_arg failed" below.
            if matches!(arg, ArgValue::Workspace(_)) {
                panic!(
                    "fixture `{}`: encode_arg vectors must not use a Workspace arg (its id is assigned during builder \
                     composition, not standalone-encodable)",
                    fixture.name
                );
            }
            let encoded =
                encode_arg(arg).unwrap_or_else(|e| panic!("fixture `{}`: encode_arg failed: {e}", fixture.name));
            // Every first-cut ArgValue (sans Workspace) encodes to an InstructionArg::Literal whose
            // bytes ARE the engine wire encoding — that is what the host must reproduce byte-for-byte.
            let bytes = encoded.as_literal_bytes().unwrap_or_else(|| {
                panic!(
                    "fixture `{}`: encode_arg vector expects a literal carrier",
                    fixture.name
                )
            });
            ExpectedOutput {
                encoded_arg_bytes: hex::encode(bytes),
                ..Default::default()
            }
        },
        OP_COSIGN_SEAL_WITH_AUTH => {
            let (record, seal_pk, signer_secret) = cosign_authorize_inputs(fixture);
            let auth = add_signature(&record, &seal_pk, &signer_secret)
                .unwrap_or_else(|e| panic!("fixture `{}`: cosign add_signature failed: {e}", fixture.name));
            let keys = fixture
                .input
                .keys
                .as_ref()
                .unwrap_or_else(|| panic!("fixture `{}`: cosign seal requires `input.keys`", fixture.name));
            let out = seal_and_encode_with_auth(
                cosign_resolved_partial(fixture),
                &keys.to_core(),
                std::slice::from_ref(&auth),
            )
            .unwrap_or_else(|e| {
                panic!(
                    "fixture `{}`: cosign seal_and_encode_with_auth failed: {e}",
                    fixture.name
                )
            });
            ExpectedOutput {
                sealed_transaction_semantic: Some(decoded_semantic_transaction(&fixture.name, &out)),
                ..Default::default()
            }
        },
        other => panic!("fixture `{}`: unknown operation `{other}`", fixture.name),
    }
}

/// Resolves a co-sign fixture's `network`/`intent`/`fetched` into a fully-resolved partial (party A's
/// transaction). Panics with a clear message if a required field is absent or the batch is insufficient.
fn cosign_resolved_partial(fixture: &Fixture) -> ootle_sdk_core::PartialTransaction {
    let input = &fixture.input;
    let network = input
        .network
        .unwrap_or_else(|| panic!("fixture `{}`: cosign op requires `input.network`", fixture.name));
    let intent = input
        .intent
        .as_ref()
        .unwrap_or_else(|| panic!("fixture `{}`: cosign op requires `input.intent`", fixture.name));
    let fetched = input
        .fetched
        .as_deref()
        .unwrap_or_else(|| panic!("fixture `{}`: cosign op requires `input.fetched`", fixture.name));
    let (partial, _wants) = ootle_sdk_core::build_public_transfer_unsigned_with_wants(network, intent)
        .unwrap_or_else(|e| panic!("fixture `{}`: build-with-wants failed: {e}", fixture.name));
    match ootle_sdk_core::apply_fetched_substates(partial, fetched)
        .unwrap_or_else(|e| panic!("fixture `{}`: apply failed: {e}", fixture.name))
    {
        ootle_sdk_core::Resolution::Resolved(p) => p,
        ootle_sdk_core::Resolution::NeedMore { want_list, .. } => panic!(
            "fixture `{}`: cosign fetched batch left {} want(s) outstanding",
            fixture.name,
            want_list.0.len()
        ),
    }
}

/// Gathers the co-sign authorize inputs: A's resolved unsigned record + A's seal public key + B's
/// co-signer secret. B authorizes with a random nonce.
fn cosign_authorize_inputs(
    fixture: &Fixture,
) -> (
    ootle_sdk_core::UnsignedTransactionRecord,
    PublicKeyBytes,
    SecretKeyBytes,
) {
    let record = ootle_sdk_core::unsigned_record_for_cosign(&cosign_resolved_partial(fixture))
        .unwrap_or_else(|e| panic!("fixture `{}`: unsigned_record_for_cosign failed: {e}", fixture.name));
    let seal_pk_hex = fixture
        .input
        .cosign_seal_pk
        .as_ref()
        .unwrap_or_else(|| panic!("fixture `{}`: cosign op requires `input.cosign_seal_pk`", fixture.name));
    let seal_pk = PublicKeyBytes::from_hex(seal_pk_hex)
        .unwrap_or_else(|e| panic!("fixture `{}`: bad cosign_seal_pk hex: {e}", fixture.name));
    let signer_secret = fixture.input.cosign_signer_secret.clone().unwrap_or_else(|| {
        panic!(
            "fixture `{}`: cosign op requires `input.cosign_signer_secret`",
            fixture.name
        )
    });
    (record, seal_pk, signer_secret)
}

/// Decodes a keygen fixture's lowercase-hex `input.seed` into a fixed 32-byte seed (panics with a clear
/// message if absent / not 32 bytes — fixtures author it deterministically).
fn seed_from_fixture(fixture: &Fixture) -> [u8; 32] {
    let seed_hex = fixture
        .input
        .seed
        .as_deref()
        .unwrap_or_else(|| panic!("fixture `{}`: keygen op requires `input.seed`", fixture.name));
    let bytes = hex::decode(seed_hex).unwrap_or_else(|e| panic!("fixture `{}`: invalid seed hex: {e}", fixture.name));
    bytes
        .try_into()
        .unwrap_or_else(|v: Vec<u8>| panic!("fixture `{}`: seed must be 32 bytes, got {}", fixture.name, v.len()))
}

/// Decodes the sealed BOR bytes back into a `Transaction`, verifies every signature, serializes it to
/// JSON, and recursively nulls the byte-unstable fields: the aggregated bulletproof
/// (`agg_range_proof`), the balance-proof (`balance_proof`), and every Schnorr signature scalar (the
/// `signature` sub-object inside each `TransactionSignature`/`TransactionSealSignature`). The signer
/// **public keys** survive, so the comparison still locks the key-selection contract (which key seals
/// / authorizes).
///
/// This delegates to the single shared canonicalizer
/// [`ootle_sdk_core::decode_and_canonicalize_sealed_transfer`], so the null set has one definition
/// in the core (`stealth::canonicalize::UNSTABLE_NULL_SET`) and is never re-implemented here. The
/// runner only adds key-sorting ([`canonicalize_json`]) for an order-insensitive compare.
fn decoded_semantic_transaction(name: &str, out: &ootle_sdk_core::EncodedPublicTransfer) -> serde_json::Value {
    let value = ootle_sdk_core::decode_and_canonicalize_sealed_transfer(
        ootle_sdk_core::types::network::Network::Esmeralda,
        &out.encoded_transaction.to_hex(),
    )
    .unwrap_or_else(|e| panic!("fixture `{name}`: sealed bytes must decode + verify: {e}"));
    canonicalize_json(value)
}

/// Recursively sorts object keys so two structurally-equal JSON values compare equal regardless of
/// key order. Parse vectors compare canonicalized structure (not raw bytes), so this removes the only
/// source of spurious diffs (serde object-key ordering).
pub fn canonicalize_json(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut sorted = serde_json::Map::new();
            let mut entries: Vec<(String, serde_json::Value)> = map.into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            for (k, v) in entries {
                sorted.insert(k, canonicalize_json(v));
            }
            serde_json::Value::Object(sorted)
        },
        serde_json::Value::Array(items) => serde_json::Value::Array(items.into_iter().map(canonicalize_json).collect()),
        other => other,
    }
}

/// The absolute path to the committed `fixtures/` directory.
pub fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

/// Loads every `*.json` fixture under `fixtures/` (recursively), sorted by path for stable ordering.
///
/// `fixtures/README.md` is skipped (only `*.json` files are fixtures).
pub fn load_all_fixtures() -> Vec<(PathBuf, Fixture)> {
    let mut paths = Vec::new();
    collect_json(&fixtures_dir(), &mut paths);
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let raw = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
            let fixture: Fixture =
                serde_json::from_str(&raw).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()));
            (path, fixture)
        })
        .collect()
}

/// Recursively collects `*.json` files under `dir` into `out`.
fn collect_json(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_json(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("json") {
            out.push(path);
        } else {
            // Non-JSON file (e.g. a README) — skip.
        }
    }
}

/// Serializes a fixture to pretty JSON with a trailing newline (stable, diff-friendly, idempotent).
pub fn fixture_to_pretty_json(fixture: &Fixture) -> String {
    let mut s = serde_json::to_string_pretty(fixture).expect("fixture serializes");
    s.push('\n');
    s
}

/// Builds the provenance block at generation time, resolving `git_rev` from the environment / git.
///
/// `git_rev` comes from `OOTLE_FIXTURE_GIT_REV` if set, else `git rev-parse HEAD`, else `"unknown"`
/// (so generation never fails on a missing git).
pub fn current_provenance() -> Provenance {
    provenance_with_rev(&git_rev())
}

/// Builds the provenance block with an **explicit** `git_rev`.
///
/// Used by hermetic tests (e.g. the idempotency check) to pin the rev *without* mutating the process
/// environment — `std::env::set_var` is a data race with the parallel test harness, so we thread the
/// override as a parameter instead.
pub fn provenance_with_rev(git_rev: &str) -> Provenance {
    Provenance {
        core_version: env!("CARGO_PKG_VERSION").to_string(),
        git_rev: git_rev.to_string(),
        generated_by: "ootle_sdk_core golden-vector generator".to_string(),
    }
}

/// Resolves the git revision for provenance (env override → `git rev-parse HEAD` → `"unknown"`).
fn git_rev() -> String {
    if let Ok(rev) = std::env::var("OOTLE_FIXTURE_GIT_REV") {
        let rev = rev.trim();
        if !rev.is_empty() {
            return rev.to_string();
        }
    }
    Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

/// Regenerates one vector: re-runs the core over `input`, refreshes `expected` + `provenance`, and
/// returns the would-be JSON bytes. Does **not** write — the caller decides (so regen can be a
/// dry-run / idempotency check too). Provenance `git_rev` is resolved from the environment / git.
pub fn regenerate(fixture: Fixture) -> (Fixture, String) {
    regenerate_with_provenance(fixture, current_provenance())
}

/// Like [`regenerate`] but with an **explicit** provenance — lets hermetic tests pin `git_rev`
/// without mutating the process environment (which would race the parallel test harness).
pub fn regenerate_with_provenance(mut fixture: Fixture, provenance: Provenance) -> (Fixture, String) {
    fixture.schema_version = SCHEMA_VERSION;
    fixture.expected = run_operation(&fixture);
    fixture.provenance = provenance;
    let json = fixture_to_pretty_json(&fixture);
    (fixture, json)
}
