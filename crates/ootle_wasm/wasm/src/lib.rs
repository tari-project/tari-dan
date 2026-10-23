//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use js_sys::BigInt;
use wasm_bindgen::prelude::*;

/// Converts a JS `bigint` amount argument to a `u64`, rejecting any value outside `[0, 2^64 - 1]`.
///
/// Exports take amounts as `BigInt` because wasm-bindgen passes a `u64` parameter as a wasm `i64`, which
/// the JS engine fills by reducing the caller's `bigint` modulo 2^64 before Rust ever sees it.
fn u64_arg(name: &str, value: BigInt) -> Result<u64, JsError> {
    u64::try_from(value).map_err(|_| JsError::new(&format!("{name} must be an integer in [0, 2^64 - 1]")))
}

/// Called automatically when the WASM module is instantiated. Do not call directly.
#[wasm_bindgen(start)]
fn on_start() {
    #[cfg(feature = "debug")]
    console_error_panic_hook::set_once();
}

/// A generated keypair (raw bytes).
#[wasm_bindgen(getter_with_clone)]
pub struct KeypairResult {
    pub secret_key: Vec<u8>,
    pub public_key: Vec<u8>,
}

/// Result of a Schnorr signature operation (raw bytes). Also used for balance proof signatures, which
/// share the `(public_nonce, signature)` shape.
#[wasm_bindgen(getter_with_clone)]
pub struct SchnorrSignatureResult {
    pub public_nonce: Vec<u8>,
    pub signature: Vec<u8>,
}

/// Result of generating a stealth outputs statement.
#[wasm_bindgen(getter_with_clone)]
pub struct StealthOutputsResult {
    /// JSON-serialized `StealthOutputsStatement` (the wire-format payload).
    pub statement_json: String,
    /// Sum of all witness masks, suitable for use as the `aggregated_output_mask` argument to
    /// `generateStealthBalanceProofSignature`.
    pub aggregated_output_mask: Vec<u8>,
}

/// Decrypted contents of an inbound stealth UTXO.
#[wasm_bindgen(getter_with_clone)]
pub struct DecryptedOutputResult {
    /// The 32-byte commitment mask scalar.
    pub mask: Vec<u8>,
    /// The plaintext value (u64).
    pub value: u64,
    /// JSON-encoded `Memo` (variants: `U256` / `Message` / `Bytes` / `PayRefAndBytes`), or `null` if
    /// the payload carried no memo or `skipMemo` was set.
    pub memo_json: Option<String>,
}

/// A pair of Ootle secret keys (owner + view).
#[wasm_bindgen(getter_with_clone)]
pub struct OotleSecretKey {
    /// The owner (spending) secret key bytes.
    pub owner_key: Vec<u8>,
    /// The view-only secret key bytes.
    pub view_key: Vec<u8>,
}

/// A pair of Ootle public keys derived from an OotleSecretKey.
#[wasm_bindgen(getter_with_clone)]
pub struct OotlePublicKey {
    /// The owner (spending) public key bytes.
    pub owner_key: Vec<u8>,
    /// The view-only public key bytes.
    pub view_key: Vec<u8>,
}

/// Parsed components of an Ootle address.
#[wasm_bindgen(getter_with_clone)]
pub struct ParsedOotleAddress {
    /// The owner (spending) public key bytes.
    pub owner_key: Vec<u8>,
    /// The view-only public key bytes.
    pub view_key: Vec<u8>,
    /// The network byte.
    pub network: u8,
    /// Optional pay reference / memo bytes.
    pub memo: Option<Vec<u8>>,
}

/// Seal a transaction (unsigned or unsealed JSON) with the seal signer's secret key.
///
/// Accepts either an `UnsignedTransactionV1` or `UnsealedTransactionV1` JSON string.
/// Returns the sealed `Transaction` as a JSON string.
#[wasm_bindgen(js_name = "sealTransaction")]
pub fn seal_transaction(tx_json: &str, seal_signer_secret_key: &[u8]) -> Result<String, JsError> {
    ootle_wasm_core::transaction::seal_transaction_json(tx_json, seal_signer_secret_key)
        .map_err(|e| JsError::new(&e.to_string()))
}

/// Add a signer to a transaction (unsigned or unsealed JSON).
///
/// Accepts either an `UnsignedTransactionV1` or `UnsealedTransactionV1` JSON string.
/// Returns the `UnsealedTransactionV1` (with the new signature appended) as a JSON string.
#[wasm_bindgen(js_name = "addTransactionSigner")]
pub fn add_transaction_signer(
    tx_json: &str,
    signer_secret_key: &[u8],
    seal_signer_public_key: &[u8],
) -> Result<String, JsError> {
    ootle_wasm_core::transaction::add_transaction_signer_json(tx_json, signer_secret_key, seal_signer_public_key)
        .map_err(|e| JsError::new(&e.to_string()))
}

/// BOR-encode a Transaction (JSON string) → base64 string (TransactionEnvelope format).
#[wasm_bindgen(js_name = "borEncodeTransaction")]
pub fn bor_encode_transaction(transaction_json: &str) -> Result<String, JsError> {
    ootle_wasm_core::bor::bor_encode_transaction_json(transaction_json).map_err(|e| JsError::new(&e.to_string()))
}

/// Schnorr-sign a message with a secret key.
/// Returns { public_nonce: Uint8Array, signature: Uint8Array }.
#[wasm_bindgen(js_name = "schnorrSign")]
pub fn schnorr_sign(secret_key: &[u8], message: &[u8]) -> Result<SchnorrSignatureResult, JsError> {
    let result = ootle_wasm_core::sign::schnorr_sign(secret_key, message).map_err(|e| JsError::new(&e.to_string()))?;
    Ok(SchnorrSignatureResult {
        public_nonce: result.public_nonce,
        signature: result.signature,
    })
}

/// Derive the public key from a secret key (both raw bytes).
#[wasm_bindgen(js_name = "publicKeyFromSecretKey")]
pub fn public_key_from_secret_key(secret_key: &[u8]) -> Result<Vec<u8>, JsError> {
    ootle_wasm_core::sign::public_key_from_secret_key(secret_key).map_err(|e| JsError::new(&e.to_string()))
}

/// Generate a new random Ristretto keypair.
/// Returns { secret_key: Uint8Array, public_key: Uint8Array }.
#[wasm_bindgen(js_name = "generateKeypair")]
pub fn generate_keypair() -> KeypairResult {
    let result = ootle_wasm_core::sign::generate_keypair();
    KeypairResult {
        secret_key: result.secret_key,
        public_key: result.public_key,
    }
}

/// Hash an UnsignedTransactionV1 (JSON string) for signing.
/// Returns the 64-byte signing message that must be Schnorr-signed.
///
/// `seal_signer_public_key` is the raw bytes of the seal signer's public key (account owner).
#[wasm_bindgen(js_name = "hashUnsignedTransaction")]
pub fn hash_unsigned_transaction(unsigned_tx_json: &str, seal_signer_public_key: &[u8]) -> Result<Vec<u8>, JsError> {
    ootle_wasm_core::hash::hash_unsigned_transaction_json(unsigned_tx_json, seal_signer_public_key)
        .map_err(|e| JsError::new(&e.to_string()))
}

/// Generate a new random pair of Ootle secret keys (owner + view).
/// Returns { owner_key: Uint8Array, view_key: Uint8Array }.
#[wasm_bindgen(js_name = "generateOotleSecretKey")]
pub fn generate_ootle_secret_key() -> OotleSecretKey {
    let result = ootle_wasm_core::address::generate_ootle_secret_key();
    OotleSecretKey {
        owner_key: result.owner_key,
        view_key: result.view_key,
    }
}

/// Derive the Ootle public keys from a pair of secret keys.
/// Returns { owner_key: Uint8Array, view_key: Uint8Array }.
#[wasm_bindgen(js_name = "ootlePublicKeyFromSecretKey")]
pub fn ootle_public_key_from_secret_key(owner_key: &[u8], view_key: &[u8]) -> Result<OotlePublicKey, JsError> {
    let secret = ootle_wasm_core::address::OotleSecretKeyResult {
        owner_key: owner_key.to_vec(),
        view_key: view_key.to_vec(),
    };
    let result = ootle_wasm_core::address::ootle_public_key_from_secret_key(&secret)
        .map_err(|e| JsError::new(&e.to_string()))?;
    Ok(OotlePublicKey {
        owner_key: result.owner_key,
        view_key: result.view_key,
    })
}

/// Generate an Ootle address (bech32m string) from public keys.
///
/// `network` is the network byte (0x00 = MainNet, 0x10 = LocalNet, 0x26 = Esmeralda, etc.).
/// `memo` is an optional pay reference (max 64 bytes).
#[wasm_bindgen(js_name = "generateOotleAddress")]
pub fn generate_ootle_address(
    owner_public_key: &[u8],
    view_public_key: &[u8],
    network: u8,
    memo: Option<Vec<u8>>,
) -> Result<String, JsError> {
    ootle_wasm_core::address::generate_ootle_address(owner_public_key, view_public_key, network, memo.as_deref())
        .map_err(|e| JsError::new(&e.to_string()))
}

/// Parse a bech32m Ootle address string into its components.
/// Returns { owner_key: Uint8Array, view_key: Uint8Array, network: number, memo: Uint8Array | undefined }.
#[wasm_bindgen(js_name = "parseOotleAddress")]
pub fn parse_ootle_address(address: &str) -> Result<ParsedOotleAddress, JsError> {
    let result = ootle_wasm_core::address::parse_ootle_address(address).map_err(|e| JsError::new(&e.to_string()))?;
    Ok(ParsedOotleAddress {
        owner_key: result.owner_key,
        view_key: result.view_key,
        network: result.network,
        memo: result.memo,
    })
}

// ---------------------------------------------------------------------------
// Stealth crypto primitives
// ---------------------------------------------------------------------------

/// Generate the output side of a stealth transfer: per-output Pedersen commitments and encrypted data,
/// optional ElGamal viewable-balance proofs (for outputs with a `resource_view_key`), and an aggregated
/// bulletproof range proof.
///
/// `witnesses_json` is a JSON array of stealth output witnesses. Each entry has the shape:
/// ```text
/// {
///   "witness": {
///     "amount": <u64>,
///     "mask": <hex 32 bytes>,
///     "sender_public_nonce": <hex 32 bytes>,
///     "minimum_value_promise": <u64>,
///     "encrypted_data": <hex variable-length>,
///     "resource_view_key": <hex 32 bytes | null>
///   },
///   "auth": <SpendAuthorization>,
///   "tag": <u32>
/// }
/// ```
///
/// Returns the serialized statement plus the aggregated output mask, which the sender feeds to
/// `generateStealthBalanceProofSignature` together with the aggregated input mask.
#[wasm_bindgen(js_name = "generateStealthOutputsStatement")]
pub fn generate_stealth_outputs_statement(
    witnesses_json: &str,
    revealed_output_amount_microtari: BigInt,
    revealed_receiver: &[u8],
) -> Result<StealthOutputsResult, JsError> {
    let result = ootle_wasm_core::stealth::outputs::generate_stealth_outputs_statement(
        witnesses_json,
        u64_arg("revealed_output_amount_microtari", revealed_output_amount_microtari)?,
        revealed_receiver,
    )
    .map_err(|e| JsError::new(&e.to_string()))?;
    Ok(StealthOutputsResult {
        statement_json: result.statement_json,
        aggregated_output_mask: result.aggregated_output_mask,
    })
}

/// Build a single stealth output witness entirely client-side (sender side), mirroring the wallet
/// daemon's `create_output_witness`.
///
/// A fresh commitment mask and ephemeral nonce are generated internally; the recipient recovers the
/// value and mask by decrypting `encrypted_data`. Returns one witness as a JSON string with the shape:
/// ```text
/// {
///   "witness": {
///     "amount": <u64>,
///     "mask": <hex 32 bytes>,
///     "sender_public_nonce": <hex 32 bytes>,
///     "minimum_value_promise": <u64>,
///     "encrypted_data": <hex variable-length>,
///     "resource_view_key": <hex 32 bytes | null>
///   },
///   "auth": <SpendAuthorization>,
///   "tag": <u32>
/// }
/// ```
/// Collect one witness per output (including change) into a JSON array and pass it to
/// `generateStealthOutputsStatement`.
///
/// - `network` is the network byte (0x00 = MainNet, 0x10 = LocalNet, 0x26 = Esmeralda, ...).
/// - `destination_account_public_key` / `destination_view_public_key` are the recipient's 32-byte keys.
/// - `resource_address` is the `resource_<hex>` string of the resource being sent.
/// - `resource_view_key` is the resource view-key holder's 32-byte public key, or `null` for resources without a
///   viewable balance (when set, the output receives an ElGamal proof at statement time).
/// - `memo_json` is an optional JSON-encoded `Memo` to embed in the encrypted payload.
/// - `pay_to_json` is an optional JSON-encoded `PayTo`: `"StealthPublicKey"` (the default when `null`, producing a
///   one-time stealth spend key) or `{"AccessRule": <AccessRule>}`.
/// - `minimum_value_promise` is the range-proof lower bound and must be `<= amount` (use `0` normally).
// `#[wasm_bindgen]` exports must take primitive/JS-marshalled arguments, so the params cannot be passed
// as a struct here — they are repacked into `CreateStealthOutputWitnessParams` for the core call.
#[wasm_bindgen(js_name = "createStealthOutputWitness")]
#[allow(clippy::too_many_arguments)]
pub fn create_stealth_output_witness(
    network: u8,
    destination_account_public_key: &[u8],
    destination_view_public_key: &[u8],
    amount: BigInt,
    resource_address: &str,
    resource_view_key: Option<Vec<u8>>,
    memo_json: Option<String>,
    pay_to_json: Option<String>,
    minimum_value_promise: BigInt,
) -> Result<String, JsError> {
    ootle_wasm_core::stealth::outputs::create_stealth_output_witness(
        ootle_wasm_core::stealth::outputs::CreateStealthOutputWitnessParams {
            network,
            destination_account_public_key,
            destination_view_public_key,
            amount: u64_arg("amount", amount)?,
            resource_address,
            resource_view_key: resource_view_key.as_deref(),
            memo_json: memo_json.as_deref(),
            pay_to_json: pay_to_json.as_deref(),
            minimum_value_promise: u64_arg("minimum_value_promise", minimum_value_promise)?,
        },
    )
    .map_err(|e| JsError::new(&e.to_string()))
}

/// Build a `StealthInputsStatement` JSON from raw input commitments and a revealed amount.
///
/// `input_commitments` is the concatenated bytes of all 32-byte commitments (so the length must be a
/// multiple of 32). Pass an empty array for a revealed-only statement.
///
/// This is a convenience helper so callers don't need to hand-craft the wire JSON; the result is used
/// as the `inputs_statement_json` argument to `generateStealthBalanceProofSignature` and friends.
#[wasm_bindgen(js_name = "buildStealthInputsStatement")]
pub fn build_stealth_inputs_statement(
    input_commitments: &[u8],
    revealed_amount_microtari: BigInt,
) -> Result<String, JsError> {
    ootle_wasm_core::stealth::inputs::build_stealth_inputs_statement(
        input_commitments,
        u64_arg("revealed_amount_microtari", revealed_amount_microtari)?,
    )
    .map_err(|e| JsError::new(&e.to_string()))
}

/// Build a `StealthInputsStatement` JSON from a JSON array of per-input `{commitment, witness}` pairs
/// and a revealed amount.
///
/// Unlike `buildStealthInputsStatement`, which only builds key-path inputs from raw commitment bytes,
/// this accepts a caller-supplied `witness` per input -- the only way to spend an output committed with
/// `PayTo::Conditions` (e.g. claiming or refunding an HTLC-style hashlock/timelock output). Build each
/// script-path input's witness with `buildScriptPathWitness` first; a plain key-path input can still be
/// included in the same call with `"witness":"KeyPath"`.
///
/// `inputs_json` shape: `[{ "commitment": <hex 32 bytes>, "witness": <SpendWitness> }, ...]`.
#[wasm_bindgen(js_name = "buildStealthInputsStatementFromInputs")]
pub fn build_stealth_inputs_statement_from_inputs(
    inputs_json: &str,
    revealed_amount_microtari: BigInt,
) -> Result<String, JsError> {
    ootle_wasm_core::stealth::inputs::build_stealth_inputs_statement_from_inputs(
        inputs_json,
        u64_arg("revealed_amount_microtari", revealed_amount_microtari)?,
    )
    .map_err(|e| JsError::new(&e.to_string()))
}

/// Build a script-path `SpendWitness` revealing `leaf` from the committed `conditions` set, optionally
/// supplying a witness `data` blob the leaf's predicate interprets (e.g. a hashlock preimage). Returns a
/// JSON object: `{ "witness": <SpendWitness>, "condition_root": <Hash32> }`.
///
/// `conditions_json` is the JSON array of `SpendCondition` leaves exactly as passed to
/// `createStealthOutputWitness`'s `PayTo::Conditions` when the output was created; `leaf_json` is the
/// single leaf being revealed. The returned `condition_root` must match the `Script` root recorded in
/// that output's `SpendAuthorization` -- record it against the spent input via
/// `buildStealthInputsStatementFromInputs`.
///
/// Pass an empty `data` array for a leaf whose predicate needs no spender-supplied data (e.g. a plain
/// timelock refund).
#[wasm_bindgen(js_name = "buildScriptPathWitness")]
pub fn build_script_path_witness(conditions_json: &str, leaf_json: &str, data: &[u8]) -> Result<String, JsError> {
    ootle_wasm_core::stealth::inputs::build_script_path_witness(conditions_json, leaf_json, data)
        .map_err(|e| JsError::new(&e.to_string()))
}

/// Build a complete `StealthTransferStatement` JSON from unblinded input/output witnesses.
///
/// Unlike `generateStealthOutputsStatement` / `buildStealthInputsStatementFromInputs` +
/// `generateStealthBalanceProofSignature`, which build and sign each half of a transfer
/// separately, this wraps the single primitive that produces the *entire*, internally-consistent
/// statement in one call, including a covenant balance-integrity proof for any script-path input
/// whose revealed leaf gates on `Covenant::BalancePreserved` (see `ootle_wasm_core::stealth::
/// transfer::build_stealth_transfer_statement` for when a claim is and isn't needed).
///
/// `input_witnesses_json` is a JSON array of `{ "mask_and_value": { "value": <u64>, "mask": <hex
/// 32 bytes> }, "witness"?: <SpendWitness>, "condition_root"?: <Hash32> }` -- each entry's
/// `witness`/`condition_root` are the exact pair `buildScriptPathWitness` returns, for a
/// script-path input; omit both for a plain key-path input. Mixing key-path and script-path
/// inputs in one call is supported.
///
/// `output_witnesses_json` is a JSON array of the same `{ "witness": {...}, "auth": ..., "tag":
/// ... }` shape `createStealthOutputWitness` returns -- collect one entry per output (including
/// change).
#[wasm_bindgen(js_name = "buildStealthTransferStatement")]
pub fn build_stealth_transfer_statement(
    input_witnesses_json: &str,
    revealed_input_amount_microtari: BigInt,
    output_witnesses_json: &str,
    revealed_output_amount_microtari: BigInt,
    revealed_receiver: &[u8],
) -> Result<String, JsError> {
    ootle_wasm_core::stealth::transfer::build_stealth_transfer_statement(
        input_witnesses_json,
        u64_arg("revealed_input_amount_microtari", revealed_input_amount_microtari)?,
        output_witnesses_json,
        u64_arg("revealed_output_amount_microtari", revealed_output_amount_microtari)?,
        revealed_receiver,
    )
    .map_err(|e| JsError::new(&e.to_string()))
}

/// Aggregate the commitment masks of stealth inputs into a single 32-byte Ristretto scalar.
///
/// `masks_concat` is the concatenated bytes of all input masks (32 bytes per mask, so the input
/// length must be a multiple of 32). Pass an empty array to obtain the zero scalar.
///
/// Returns the sum as 32 bytes, suitable as the `aggregated_input_mask` argument to
/// `generateStealthBalanceProofSignature`. The output side of the same balance proof is aggregated
/// automatically by `generateStealthOutputsStatement` (returned as `aggregated_output_mask`).
#[wasm_bindgen(js_name = "aggregateInputMasks")]
pub fn aggregate_input_masks(masks_concat: &[u8]) -> Result<Vec<u8>, JsError> {
    ootle_wasm_core::stealth::inputs::aggregate_input_masks(masks_concat).map_err(|e| JsError::new(&e.to_string()))
}

/// Generate an extended bulletproof aggregating range proofs for a set of output witnesses, proving
/// each amount is in `[minimum_value_promise, 2^64)`. The number of witnesses is padded to the next
/// power of two internally.
///
/// `witnesses_json` is a JSON array of "flat" output witnesses (the `witness` field shape from
/// [`generate_stealth_outputs_statement`] — without the surrounding `auth` / `tag`).
///
/// Returns the raw range proof bytes (may be empty if the input array is empty).
#[wasm_bindgen(js_name = "generateExtendedBulletProof")]
pub fn generate_extended_bullet_proof(witnesses_json: &str) -> Result<Vec<u8>, JsError> {
    ootle_wasm_core::stealth::outputs::generate_extended_bullet_proof(witnesses_json)
        .map_err(|e| JsError::new(&e.to_string()))
}

/// Sign the balance proof for a stealth transfer.
///
/// `aggregated_input_mask` and `aggregated_output_mask` are the 32-byte sums of all input / output
/// commitment masks respectively. Returns a `(public_nonce, signature)` pair (each 32 bytes); the pair
/// may be all-zeros for revealed-only transfers — callers normally omit the balance proof in that case.
#[wasm_bindgen(js_name = "generateStealthBalanceProofSignature")]
pub fn generate_stealth_balance_proof_signature(
    aggregated_input_mask: &[u8],
    aggregated_output_mask: &[u8],
    inputs_statement_json: &str,
    outputs_statement_json: &str,
    covenant_claims_json: &str,
) -> Result<SchnorrSignatureResult, JsError> {
    let result = ootle_wasm_core::stealth::balance_proof::generate_stealth_balance_proof_signature(
        aggregated_input_mask,
        aggregated_output_mask,
        inputs_statement_json,
        outputs_statement_json,
        covenant_claims_json,
    )
    .map_err(|e| JsError::new(&e.to_string()))?;
    Ok(SchnorrSignatureResult {
        public_nonce: result.public_nonce,
        signature: result.signature,
    })
}

/// Pre-flight check that a balance proof signature is cryptographically valid for the given input /
/// output statements. Returns `false` on a malformed proof or invalid signature; the engine performs
/// the authoritative check at submission.
#[wasm_bindgen(js_name = "validateBalanceProofSignature")]
pub fn validate_balance_proof_signature(
    public_nonce: &[u8],
    signature: &[u8],
    inputs_statement_json: &str,
    outputs_statement_json: &str,
    covenant_claims_json: &str,
) -> Result<bool, JsError> {
    ootle_wasm_core::stealth::balance_proof::validate_balance_proof_signature(
        public_nonce,
        signature,
        inputs_statement_json,
        outputs_statement_json,
        covenant_claims_json,
    )
    .map_err(|e| JsError::new(&e.to_string()))
}

/// Run the same validation the engine performs on a complete `StealthTransferStatement` envelope:
/// structural sanity, commitment well-formedness, range and balance-proof verification.
///
/// `view_key` is the 32-byte resource view public key, required for resources with a viewable balance
/// and rejected otherwise. Pass `null` for resources without a view key.
///
/// Throws on a validation failure; returns successfully on a valid statement.
#[wasm_bindgen(js_name = "validateStealthTransfer")]
pub fn validate_stealth_transfer(transfer_json: &str, view_key: Option<Vec<u8>>) -> Result<(), JsError> {
    ootle_wasm_core::stealth::validate::validate_stealth_transfer(transfer_json, view_key.as_deref())
        .map_err(|e| JsError::new(&e.to_string()))
}

/// Decrypt and verify the AEAD payload of an inbound stealth UTXO.
///
/// `output_commitment` is the 32-byte Pedersen commitment; `encrypted_data` is the variable-length
/// XChaCha20Poly1305-encrypted blob; `encryption_key` is the 32-byte AEAD key derived via
/// `encryptedDataDhKdfAead`. Setting `skip_memo` to `true` returns no memo even if the payload carries
/// one (useful when only the value / mask are needed).
///
/// Throws on AEAD failure or on a commitment mismatch — either indicates the payload was not produced
/// for this view key.
#[wasm_bindgen(js_name = "unblindOutput")]
pub fn unblind_output(
    output_commitment: &[u8],
    encrypted_data: &[u8],
    encryption_key: &[u8],
    skip_memo: bool,
) -> Result<DecryptedOutputResult, JsError> {
    let result = ootle_wasm_core::stealth::encrypted_data::unblind_output(
        output_commitment,
        encrypted_data,
        encryption_key,
        skip_memo,
    )
    .map_err(|e| JsError::new(&e.to_string()))?;
    Ok(DecryptedOutputResult {
        mask: result.mask,
        value: result.value,
        memo_json: result.memo_json,
    })
}

/// Derive the recipient's stealth spending scalar `c + k`, where `c = H(network || k.G * r)`. The
/// receiver runs this with their account secret key (`private_key`) and the sender-provided public
/// nonce to obtain the one-time secret that controls the stealth output.
///
/// `network` is the network byte (0x00 = MainNet, 0x10 = LocalNet, 0x26 = Esmeralda, ...).
#[wasm_bindgen(js_name = "stealthDhSecret")]
pub fn stealth_dh_secret(network: u8, private_key: &[u8], public_nonce: &[u8]) -> Result<Vec<u8>, JsError> {
    ootle_wasm_core::stealth::kdfs::stealth_dh_secret(network, private_key, public_nonce)
        .map_err(|e| JsError::new(&e.to_string()))
}

/// Derive the AEAD encryption key for `encrypted_data` from a Diffie-Hellman shared secret: `H(DH(s, P))`.
/// Sender derives it with `(sender_secret_nonce, recipient_view_pub)`; receiver derives the same key
/// with `(recipient_view_secret, sender_public_nonce)`.
#[wasm_bindgen(js_name = "encryptedDataDhKdfAead")]
pub fn encrypted_data_dh_kdf_aead(private_key: &[u8], public_key: &[u8]) -> Result<Vec<u8>, JsError> {
    ootle_wasm_core::stealth::kdfs::encrypted_data_dh_kdf(private_key, public_key)
        .map_err(|e| JsError::new(&e.to_string()))
}

/// Generate an ElGamal viewable-balance proof: a zero-knowledge proof that `amount` is the value bound
/// by `commitment`, encrypted to the resource view-key holder.
///
/// Returns the JSON-encoded `ViewableBalanceProof` (8 × 32-byte fields).
#[wasm_bindgen(js_name = "generateElgamalViewableBalanceProof")]
pub fn generate_elgamal_viewable_balance_proof(
    mask: &[u8],
    amount: BigInt,
    commitment: &[u8],
    view_public_key: &[u8],
) -> Result<String, JsError> {
    ootle_wasm_core::stealth::viewable_balance::generate_elgamal_viewable_balance_proof(
        mask,
        u64_arg("amount", amount)?,
        commitment,
        view_public_key,
    )
    .map_err(|e| JsError::new(&e.to_string()))
}

/// Brute-force decrypt an ElGamal viewable-balance proof to recover the bound value.
///
/// Tries each value in `[min_value, max_value]` (inclusive). Returns `null` (via `Option`) if no
/// candidate matches. Uses an on-the-fly value lookup — there is no precomputed table dependency, so
/// callers should keep the range tight (large ranges produce proportional CPU cost).
///
/// `commitment` is the Pedersen commitment the proof is bound to. Both the view public key and the
/// view secret key are required: the public key is used to re-verify the ZK proof (rejecting tampered
/// proofs before decrypting), the secret key performs the ElGamal decryption itself.
#[wasm_bindgen(js_name = "decryptElgamalViewableBalance")]
pub fn decrypt_elgamal_viewable_balance(
    proof_json: &str,
    commitment: &[u8],
    view_public_key: &[u8],
    view_secret_key: &[u8],
    min_value: BigInt,
    max_value: BigInt,
) -> Result<Option<u64>, JsError> {
    ootle_wasm_core::stealth::viewable_balance::decrypt_elgamal_viewable_balance(
        proof_json,
        commitment,
        view_public_key,
        view_secret_key,
        u64_arg("min_value", min_value)?,
        u64_arg("max_value", max_value)?,
    )
    .map_err(|e| JsError::new(&e.to_string()))
}
