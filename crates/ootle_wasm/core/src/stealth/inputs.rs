//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Building the input side of a stealth transfer.
//!
//! The inputs statement is structurally trivial — `(commitments[], revealed_amount)` — but its on-wire
//! JSON layout is not something callers should have to know about. This module exposes a small builder
//! so Python (or any other client) can assemble a `StealthInputsStatement` from raw commitment bytes
//! without hand-crafting JSON.

use tari_crypto::{ristretto::RistrettoSecretKey, tari_utilities::ByteArray};
use tari_ootle_wallet_crypto::stealth::script_path_witness_with_data;
use tari_template_lib_types::{
    Amount,
    bytes::Bytes,
    crypto::PedersenCommitmentBytes,
    stealth::{SpendCondition, StealthInput, StealthInputsStatement},
};

use crate::{
    error::OotleWasmError,
    keys::{commitment_bytes_from_bytes, secret_key_from_bytes},
};

/// Build a `StealthInputsStatement` JSON from a list of raw 32-byte input commitments and a revealed
/// amount.
///
/// `input_commitments` is the concatenated bytes of all input commitments (32 bytes per commitment, so
/// the input length must be a multiple of 32). Pass an empty slice to build a revealed-only statement.
pub fn build_stealth_inputs_statement(
    input_commitments: &[u8],
    revealed_amount_microtari: u64,
) -> Result<String, OotleWasmError> {
    if !input_commitments
        .len()
        .is_multiple_of(PedersenCommitmentBytes::length())
    {
        return Err(OotleWasmError::InvalidByteLength {
            field: "input_commitments",
            expected: PedersenCommitmentBytes::length(),
            got: input_commitments.len(),
        });
    }

    let inputs = input_commitments
        .chunks_exact(PedersenCommitmentBytes::length())
        .map(|chunk| commitment_bytes_from_bytes(chunk).map(StealthInput::new))
        .collect::<Result<Vec<_>, _>>()?;

    let statement = StealthInputsStatement {
        inputs,
        revealed_amount: Amount::from_u64(revealed_amount_microtari),
    };
    Ok(serde_json::to_string(&statement)?)
}

/// Build a `StealthInputsStatement` JSON from a JSON array of [`StealthInput`], each carrying its own
/// per-input `SpendWitness` (key path or script path), and a revealed amount.
///
/// Unlike [`build_stealth_inputs_statement`], which only ever builds key-path inputs from raw commitment
/// bytes, this accepts a caller-supplied witness per input — the only way to spend an output committed
/// with `PayTo::Conditions` (e.g. claiming or refunding an HTLC-style hashlock/timelock output). Build
/// each script-path input's witness with [`build_script_path_witness`] first; a plain key-path input can
/// still be included in the same call as `{"commitment": <hex>, "witness": "KeyPath"}`.
pub fn build_stealth_inputs_statement_from_inputs(
    inputs_json: &str,
    revealed_amount_microtari: u64,
) -> Result<String, OotleWasmError> {
    let inputs: Vec<StealthInput> = serde_json::from_str(inputs_json)?;
    let statement = StealthInputsStatement {
        inputs,
        revealed_amount: Amount::from_u64(revealed_amount_microtari),
    };
    Ok(serde_json::to_string(&statement)?)
}

/// Build a script-path [`SpendWitness`](tari_template_lib_types::stealth::SpendWitness) revealing `leaf`
/// from the committed `conditions` set, optionally supplying a witness `data` blob the leaf's predicate
/// interprets (e.g. a hashlock preimage). Returns a JSON object:
/// `{ "witness": <SpendWitness>, "condition_root": <Hash32> }`.
///
/// `conditions_json` is the JSON array of [`SpendCondition`] leaves exactly as passed to
/// `createStealthOutputWitness`'s `PayTo::Conditions`; `leaf_json` is the single leaf being revealed. The
/// returned `condition_root` must match the `Script` root recorded in the output's `SpendAuthorization` at
/// creation time — record it against the spent input (see [`build_stealth_inputs_statement_from_inputs`]'s
/// caller-supplied witness).
///
/// Pass an empty `data` slice for a leaf whose predicate needs no spender-supplied data (e.g. a plain
/// timelock).
pub fn build_script_path_witness(
    conditions_json: &str,
    leaf_json: &str,
    data: &[u8],
) -> Result<String, OotleWasmError> {
    let conditions: Vec<SpendCondition> = serde_json::from_str(conditions_json)?;
    let leaf: SpendCondition = serde_json::from_str(leaf_json)?;
    let (witness, condition_root) = script_path_witness_with_data(&conditions, &leaf, Bytes::from(data.to_vec()))
        .map_err(|e| OotleWasmError::Stealth(e.to_string()))?;
    Ok(serde_json::to_string(
        &serde_json::json!({ "witness": witness, "condition_root": condition_root }),
    )?)
}

/// Aggregate the commitment masks of stealth inputs into a single 32-byte Ristretto scalar.
///
/// `masks_concat` is the concatenated bytes of the input masks (32 bytes per mask, so the input
/// length must be a multiple of 32). Pass an empty slice to obtain the zero scalar.
///
/// Returns the sum as 32 bytes, suitable as the `aggregated_input_mask` argument to
/// [`crate::stealth::balance_proof::generate_stealth_balance_proof_signature`]. The output side of
/// the balance proof is aggregated automatically by
/// [`crate::stealth::outputs::generate_stealth_outputs_statement`].
pub fn aggregate_input_masks(masks_concat: &[u8]) -> Result<Vec<u8>, OotleWasmError> {
    const SCALAR_LEN: usize = 32;
    if !masks_concat.len().is_multiple_of(SCALAR_LEN) {
        return Err(OotleWasmError::InvalidByteLength {
            field: "masks_concat",
            expected: SCALAR_LEN,
            got: masks_concat.len(),
        });
    }

    let mut acc = RistrettoSecretKey::default();
    for chunk in masks_concat.chunks_exact(SCALAR_LEN) {
        acc = acc + secret_key_from_bytes(chunk)?;
    }
    Ok(acc.as_bytes().to_vec())
}

#[cfg(test)]
mod tests {
    use tari_crypto::keys::SecretKey;

    use super::*;

    #[test]
    fn build_revealed_only_statement() {
        let json = build_stealth_inputs_statement(&[], 1000).unwrap();
        let stmt: StealthInputsStatement = serde_json::from_str(&json).unwrap();
        assert!(stmt.inputs.is_empty());
        assert_eq!(stmt.revealed_amount, Amount::from_u64(1000));
    }

    #[test]
    fn build_statement_with_two_inputs() {
        let commitments: Vec<u8> = (0..64).map(|i| i as u8).collect();
        let json = build_stealth_inputs_statement(&commitments, 0).unwrap();
        let stmt: StealthInputsStatement = serde_json::from_str(&json).unwrap();
        assert_eq!(stmt.inputs.len(), 2);
        assert_eq!(stmt.inputs[0].commitment.as_bytes(), &commitments[..32]);
        assert_eq!(stmt.inputs[1].commitment.as_bytes(), &commitments[32..]);
    }

    #[test]
    fn rejects_non_multiple_of_32() {
        let err = build_stealth_inputs_statement(&[0u8; 33], 0).unwrap_err();
        assert!(matches!(err, OotleWasmError::InvalidByteLength { .. }));
    }

    #[test]
    fn aggregate_empty_returns_zero_scalar() {
        let result = aggregate_input_masks(&[]).unwrap();
        assert_eq!(result, vec![0u8; 32]);
    }

    #[test]
    fn aggregate_single_mask_returns_itself() {
        let mask = RistrettoSecretKey::random(&mut rand::rng());
        let result = aggregate_input_masks(mask.as_bytes()).unwrap();
        assert_eq!(result, mask.as_bytes().to_vec());
    }

    #[test]
    fn aggregate_two_masks_matches_native_addition() {
        let mut rng = rand::rng();
        let a = RistrettoSecretKey::random(&mut rng);
        let b = RistrettoSecretKey::random(&mut rng);
        let mut concat = a.as_bytes().to_vec();
        concat.extend_from_slice(b.as_bytes());

        let sum = aggregate_input_masks(&concat).unwrap();
        let expected = (a + b).as_bytes().to_vec();
        assert_eq!(sum, expected);
    }

    #[test]
    fn aggregate_rejects_non_multiple_of_32() {
        let err = aggregate_input_masks(&[0u8; 33]).unwrap_err();
        assert!(matches!(err, OotleWasmError::InvalidByteLength {
            field: "masks_concat",
            ..
        }));
    }

    /// Two-leaf HTLC-shaped condition tree: a hashlock claim leaf and a timelock refund leaf.
    fn htlc_conditions_json(hash_hex: &str) -> String {
        format!(
            r#"[[{{"Builtin":{{"HashLock":{{"hash":"{hash_hex}","alg":"Sha256"}}}}}}],[{{"Builtin":{{"AfterEpoch":1000}}}}]]"#
        )
    }

    #[test]
    fn script_path_witness_reveals_the_claim_leaf() {
        let hash_hex = hex::encode([7u8; 32]);
        let conditions_json = htlc_conditions_json(&hash_hex);
        let claim_leaf_json = format!(r#"[{{"Builtin":{{"HashLock":{{"hash":"{hash_hex}","alg":"Sha256"}}}}}}]"#);
        let preimage = [7u8; 32];

        let result = build_script_path_witness(&conditions_json, &claim_leaf_json, &preimage).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        let witness = &parsed["witness"];
        assert!(
            witness.get("ScriptPath").is_some(),
            "expected a ScriptPath witness, got {witness}"
        );
        assert_eq!(
            witness["ScriptPath"]["data"].as_str().map(|s| s.to_string()),
            Some(hex::encode(preimage))
        );
    }

    #[test]
    fn script_path_witness_condition_root_matches_output_side_pay_to_conditions() {
        use tari_template_lib_types::stealth::SpendCondition;

        let hash_hex = hex::encode([9u8; 32]);
        let conditions_json = htlc_conditions_json(&hash_hex);
        let claim_leaf_json = format!(r#"[{{"Builtin":{{"HashLock":{{"hash":"{hash_hex}","alg":"Sha256"}}}}}}]"#);

        let result = build_script_path_witness(&conditions_json, &claim_leaf_json, &[]).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        let root_from_witness = parsed["condition_root"].as_str().unwrap().to_string();

        // Independently recompute the root the way output creation (PayTo::Conditions) does. These
        // MUST agree: a mismatch here means a script-path-authorized output could be created under one
        // root and never spendable under the witness this function produces -- a fund-loss bug, not a
        // cosmetic one.
        let conditions: Vec<SpendCondition> = serde_json::from_str(&conditions_json).unwrap();
        let expected_root = tari_ootle_wallet_crypto::stealth::validated_condition_root(&conditions).unwrap();
        assert_eq!(root_from_witness, expected_root.to_string());
    }

    #[test]
    fn script_path_witness_rejects_a_leaf_not_in_the_committed_set() {
        let hash_hex = hex::encode([1u8; 32]);
        let conditions_json = htlc_conditions_json(&hash_hex);
        let foreign_leaf_json = r#"[{"Builtin":{"AfterEpoch":999999}}]"#;

        let err = build_script_path_witness(&conditions_json, foreign_leaf_json, &[]).unwrap_err();
        assert!(matches!(err, OotleWasmError::Stealth(_)));
    }

    #[test]
    fn statement_from_inputs_carries_a_script_path_witness_through() {
        let hash_hex = hex::encode([3u8; 32]);
        let conditions_json = htlc_conditions_json(&hash_hex);
        let claim_leaf_json = format!(r#"[{{"Builtin":{{"HashLock":{{"hash":"{hash_hex}","alg":"Sha256"}}}}}}]"#);
        let witness_result = build_script_path_witness(&conditions_json, &claim_leaf_json, &[3u8; 32]).unwrap();
        let witness_value: serde_json::Value = serde_json::from_str(&witness_result).unwrap();

        let commitment_hex = hex::encode([5u8; 32]);
        let inputs_json = format!(
            r#"[{{"commitment":"{commitment_hex}","witness":{witness}}}]"#,
            witness = witness_value["witness"]
        );

        let statement_json = build_stealth_inputs_statement_from_inputs(&inputs_json, 0).unwrap();
        let statement: StealthInputsStatement = serde_json::from_str(&statement_json).unwrap();
        assert_eq!(statement.inputs.len(), 1);
        assert!(
            serde_json::to_value(&statement.inputs[0].witness)
                .unwrap()
                .get("ScriptPath")
                .is_some()
        );
    }

    #[test]
    fn statement_from_inputs_builds_a_key_path_input() {
        let commitment_hex = hex::encode([2u8; 32]);
        let inputs_json = format!(r#"[{{"commitment":"{commitment_hex}","witness":"KeyPath"}}]"#);
        let statement_json = build_stealth_inputs_statement_from_inputs(&inputs_json, 500).unwrap();
        let statement: StealthInputsStatement = serde_json::from_str(&statement_json).unwrap();
        assert_eq!(statement.inputs.len(), 1);
        assert_eq!(serde_json::to_value(&statement.inputs[0].witness).unwrap(), "KeyPath");
        assert_eq!(statement.revealed_amount, Amount::from_u64(500));
    }

    #[test]
    fn statement_from_inputs_requires_a_witness() {
        let commitment_hex = hex::encode([2u8; 32]);
        let inputs_json = format!(r#"[{{"commitment":"{commitment_hex}"}}]"#);
        build_stealth_inputs_statement_from_inputs(&inputs_json, 500).unwrap_err();
    }
}
