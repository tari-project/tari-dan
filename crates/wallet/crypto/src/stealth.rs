//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashSet;

use ootle_byte_type::ToByteType;
use tari_crypto::ristretto::RistrettoSecretKey;
/// The canonical digest for a
/// [`BuiltinPredicate::HashLock`](tari_template_lib_types::stealth::BuiltinPredicate::HashLock)
/// preimage, re-exported so a wallet can build a hashlock condition without depending on `tari_engine_types`.
pub use tari_engine_types::stealth::hashlock_digest;
use tari_engine_types::{
    limits::STEALTH_LIMITS,
    stealth::{MerkleTree, validate_condition_structure},
};
use tari_template_lib_types::{
    Amount,
    Hash32,
    bytes::Bytes,
    crypto::RistrettoPublicKeyBytes,
    stealth::{
        CovenantBalanceClaim,
        SpendAuthorization,
        SpendCondition,
        SpendWitness,
        StealthInput,
        StealthInputsStatement,
        StealthOutputsStatement,
        StealthTransferStatement,
        StealthUnspentOutput,
        UnspentOutput,
    },
};

use crate::{
    StealthInputWitness,
    StealthOutputWitness,
    WalletCryptoError,
    balance_proof::{generate_covenant_balance_proof_signature, generate_stealth_balance_proof_signature},
    bullet_proof::generate_extended_bullet_proof,
    error::StealthProofError,
    pay_to::PayTo,
    viewable_balance_proof::generate_elgamal_viewable_balance_proof,
};

/// Computes the committed condition-tree (MAST) root for a set of spend-condition leaves. The root is independent of
/// leaf order. Native hashing lives in `tari_engine_types`; this is the wallet-side entry point for building a
/// script-gated output.
///
/// This is the bare tree construction: it rejects a set that cannot form a tree, but says nothing about whether the
/// individual leaves are ones the engine will admit. Use [`validated_condition_root`] for a caller-supplied set.
pub fn condition_root(conditions: &[SpendCondition]) -> Result<Hash32, WalletCryptoError> {
    MerkleTree::from_conditions(conditions)
        .map(|tree| tree.root())
        .map_err(|e| WalletCryptoError::InvalidArgument {
            name: "conditions",
            details: e.to_string(),
        })
}

/// Computes the committed condition-tree root for a caller-supplied set, checking each leaf against the engine's
/// spend-time structural rule first.
///
/// A leaf the engine will refuse to evaluate is a spend path that can never be taken, so a tree whose leaves are all
/// inadmissible commits funds to an output nobody can spend. The wallet must not build such an output, even though
/// the tree itself is well formed.
pub fn validated_condition_root(conditions: &[SpendCondition]) -> Result<Hash32, WalletCryptoError> {
    for leaf in conditions {
        validate_condition_structure(leaf).map_err(|e| WalletCryptoError::InvalidArgument {
            name: "conditions",
            details: e.to_string(),
        })?;
    }

    condition_root(conditions)
}

/// Resolves a [`PayTo`] intent to a stealth output's [`SpendAuthorization`]. The one-time stealth key is derived lazily
/// via `derive_stealth_key`, so it is only computed for the key-path intent.
pub fn pay_to_output_authorization(
    pay_to: &PayTo,
    derive_stealth_key: impl FnOnce() -> RistrettoPublicKeyBytes,
) -> Result<SpendAuthorization, WalletCryptoError> {
    match pay_to {
        PayTo::StealthPublicKey => Ok(SpendAuthorization::Key(derive_stealth_key())),
        PayTo::AccessRule(rule) => Ok(SpendAuthorization::Script(condition_root(&[
            SpendCondition::access_rule(rule.clone()),
        ])?)),
        PayTo::TemplateFunction(tf) => Ok(SpendAuthorization::Script(condition_root(&[
            SpendCondition::template_function(tf.clone()),
        ])?)),
        PayTo::Conditions(conditions) => Ok(SpendAuthorization::Script(validated_condition_root(conditions)?)),
    }
}

/// Builds a script-path [`SpendWitness`] revealing `leaf` from the committed `conditions` set, returning it alongside
/// the committed `condition_root` to record against the spent input. Use [`script_path_witness_with_data`] when the
/// leaf has a predicate that consumes spender-supplied data (e.g. a hashlock preimage).
pub fn script_path_witness(
    conditions: &[SpendCondition],
    leaf: &SpendCondition,
) -> Result<(SpendWitness, Hash32), WalletCryptoError> {
    script_path_witness_with_data(conditions, leaf, Bytes::default())
}

/// Builds a script-path [`SpendWitness`] revealing `leaf`, supplying a witness `data` blob the leaf interprets (e.g. a
/// hashlock preimage, or a CBOR structure a `TemplateFunction` decodes). The blob must not exceed
/// `STEALTH_LIMITS.max_witness_data_len`.
pub fn script_path_witness_with_data(
    conditions: &[SpendCondition],
    leaf: &SpendCondition,
    data: Bytes,
) -> Result<(SpendWitness, Hash32), WalletCryptoError> {
    let max_witness_data_len = STEALTH_LIMITS.max_witness_data_len;
    if data.len() > max_witness_data_len {
        return Err(WalletCryptoError::InvalidArgument {
            name: "data",
            details: format!(
                "witness data is {} bytes, exceeding the limit of {max_witness_data_len}",
                data.len()
            ),
        });
    }
    let tree = MerkleTree::from_conditions(conditions).map_err(|e| WalletCryptoError::InvalidArgument {
        name: "conditions",
        details: e.to_string(),
    })?;
    let proof = tree
        .proof_for_condition(leaf)
        .ok_or_else(|| WalletCryptoError::InvalidArgument {
            name: "leaf",
            details: "Revealed leaf is not a member of the committed condition set".to_string(),
        })?;
    Ok((
        SpendWitness::script_path_with_data(leaf.clone(), proof, data),
        tree.root(),
    ))
}

pub fn create_transfer_statement<'a, Inputs, Outputs>(
    inputs: Inputs,
    revealed_input_amount: Amount,
    output_statements: Outputs,
    revealed_output_amount: Amount,
) -> Result<StealthTransferStatement, WalletCryptoError>
where
    Inputs: IntoIterator<Item = StealthInputWitness>,
    Inputs::IntoIter: ExactSizeIterator,
    Outputs: IntoIterator<Item = &'a StealthOutputWitness> + Clone,
    Outputs::IntoIter: ExactSizeIterator,
{
    if revealed_input_amount.is_negative() {
        return Err(WalletCryptoError::InvalidArgument {
            name: "revealed_input_amount",
            details: format!("Revealed input amount must be non-negative: {revealed_input_amount}"),
        });
    }
    if revealed_output_amount.is_negative() {
        return Err(WalletCryptoError::InvalidArgument {
            name: "revealed_output_amount",
            details: format!("Revealed output amount must be non-negative: {revealed_output_amount}"),
        });
    }

    let inputs = inputs.into_iter().collect::<Vec<_>>();
    let num_inputs = inputs.len();

    let outputs_statement = create_outputs_statement(output_statements.clone(), revealed_output_amount)?;
    let output_witnesses = output_statements.into_iter().collect::<Vec<_>>();
    let num_outputs = output_witnesses.len();

    let mut inputs_to_spend = Vec::with_capacity(num_inputs);
    let mut agg_input_mask = RistrettoSecretKey::default();
    let mut seen = HashSet::with_capacity(num_inputs);
    for input in &inputs {
        let commitment = input.mask_and_value.to_commitment().to_byte_type();
        // The excess and the aggregate mask both fold the inputs positionally, so a UTXO listed twice would be
        // spent twice over. Rejecting it here rather than dropping it keeps the caller's mistake legible: the
        // balance proof signs the mask difference alone, so a statement whose inputs were silently deduplicated
        // would fail at the validator as an unbalanced proof.
        if !seen.insert(commitment) {
            return Err(WalletCryptoError::InvalidArgument {
                name: "inputs",
                details: format!("Input commitment {commitment} appears more than once"),
            });
        }
        inputs_to_spend.push(StealthInput {
            commitment,
            witness: input.witness.clone(),
        });
        agg_input_mask = agg_input_mask + &input.mask_and_value.mask;
    }

    let agg_output_mask = output_witnesses
        .iter()
        .map(|stmt| &stmt.witness.mask)
        .fold(RistrettoSecretKey::default(), |agg, mask| agg + mask);

    let inputs_statement = StealthInputsStatement {
        inputs: inputs_to_spend,
        revealed_amount: revealed_input_amount,
    };

    let requires_balance_proof = num_inputs > 0 || num_outputs > 0;
    let balance_proof = requires_balance_proof.then(|| {
        generate_stealth_balance_proof_signature(
            &agg_input_mask,
            &agg_output_mask,
            &inputs_statement,
            &outputs_statement,
        )
    });

    let covenant_claims = generate_covenant_claims(&inputs, &output_witnesses)?;

    Ok(StealthTransferStatement {
        inputs_statement,
        outputs_statement,
        balance_proof,
        covenant_claims,
    })
}

/// Generates a [`CovenantBalanceClaim`] for each distinct `condition_root` among the script-path spent inputs, keyed by
/// the index of the partition's first input. Inputs and outputs are partitioned by `condition_root` (matching the
/// engine's verification), and each partition's proof attests that its committed input value equals its committed
/// output value plus the exact cleartext outflow `revealed_amount = Σ input values - Σ output values`.
///
/// A covenant partition may not receive more value than it spends in the same transaction (`revealed_amount` would be
/// negative); deposit into a covenant separately.
fn generate_covenant_claims(
    inputs: &[StealthInputWitness],
    outputs: &[&StealthOutputWitness],
) -> Result<Vec<CovenantBalanceClaim>, WalletCryptoError> {
    let mut claims = Vec::new();
    let mut seen: Vec<Hash32> = Vec::new();

    for (partition_input_index, root) in inputs
        .iter()
        .enumerate()
        .filter_map(|(index, input)| Some((index, input.condition_root?)))
    {
        if seen.contains(&root) {
            continue;
        }
        seen.push(root);

        let value_overflow = || WalletCryptoError::InvalidArgument {
            name: "covenant",
            details: "Covenant partition value sum overflowed".to_string(),
        };

        let mut agg_input_mask = RistrettoSecretKey::default();
        let mut input_value = Amount::zero();
        let mut input_commitments = Vec::new();
        for input in inputs.iter().filter(|i| i.condition_root == Some(root)) {
            agg_input_mask = agg_input_mask + &input.mask_and_value.mask;
            input_value = input_value
                .checked_add(Amount::from_u64(input.mask_and_value.value))
                .ok_or_else(value_overflow)?;
            input_commitments.push(input.mask_and_value.to_commitment().to_byte_type());
        }

        let mut agg_output_mask = RistrettoSecretKey::default();
        let mut output_value = Amount::zero();
        let mut output_commitments = Vec::new();
        // Only `Script(root)` keeps value under the covenant; a `KeyAndScript` output committing the same root is
        // still key-spendable next block, so it must not be counted as conserving the partition (matches the
        // engine's `StealthOutputView::is_locked_under`).
        for output in outputs
            .iter()
            .filter(|o| matches!(&o.auth, SpendAuthorization::Script(r) if *r == root))
        {
            agg_output_mask = agg_output_mask + &output.witness.mask;
            output_value = output_value
                .checked_add(Amount::from_u64(output.witness.amount))
                .ok_or_else(value_overflow)?;
            output_commitments.push(output.witness.to_commitment().to_byte_type());
        }

        let Some(revealed_amount) = input_value.checked_sub(output_value) else {
            return Err(WalletCryptoError::InvalidArgument {
                name: "covenant",
                details: "A covenant partition may not receive more value than it spends in the same transaction"
                    .to_string(),
            });
        };

        claims.push(CovenantBalanceClaim {
            partition_input_index: partition_input_index as u32,
            revealed_amount,
            signature: generate_covenant_balance_proof_signature(
                &root,
                &agg_input_mask,
                &agg_output_mask,
                revealed_amount,
                &input_commitments,
                &output_commitments,
            ),
        });
    }

    Ok(claims)
}

pub fn create_outputs_statement<'a, Outputs: IntoIterator<Item = &'a StealthOutputWitness> + Clone>(
    output_statements: Outputs,
    revealed_output_amount: Amount,
) -> Result<StealthOutputsStatement, StealthProofError> {
    let outputs = output_statements
        .clone()
        .into_iter()
        .map(|output_stmt| {
            let unblinded_stmt = &output_stmt.witness;
            let commitment = output_stmt.witness.to_commitment();
            let output = UnspentOutput {
                commitment: commitment.to_byte_type(),
                sender_public_nonce: unblinded_stmt.sender_public_nonce.to_byte_type(),
                encrypted_data: unblinded_stmt.encrypted_data.clone(),
                minimum_value_promise: unblinded_stmt.minimum_value_promise,
                viewable_balance_proof: unblinded_stmt
                    .resource_view_key
                    .as_ref()
                    .map(|view_key| {
                        let amount = unblinded_stmt.amount;
                        generate_elgamal_viewable_balance_proof(&unblinded_stmt.mask, amount, &commitment, view_key)
                    })
                    .transpose()?,
            };

            Ok::<_, StealthProofError>(StealthUnspentOutput {
                output,
                auth: output_stmt.auth.clone(),
                tag: output_stmt.tag,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    let output_range_proof = generate_extended_bullet_proof(output_statements.into_iter().map(|o| &o.witness))?;

    Ok(StealthOutputsStatement {
        outputs,
        revealed_output_amount,
        agg_range_proof: output_range_proof,
    })
}

#[cfg(test)]
mod tests {
    use tari_crypto::{keys::SecretKey, ristretto::RistrettoSecretKey};
    use tari_engine_types::stealth::validate_stealth_outputs_statement;
    use tari_template_lib_types::{
        Amount,
        EncryptedData,
        crypto::{RistrettoPublicKeyBytes, UtxoTag},
    };

    use super::*;
    use crate::{MaskAndValue, OutputWitness};

    fn create_valid_proof(amount: u64, minimum_value_promise: u64) -> StealthOutputsStatement {
        let mask = RistrettoSecretKey::random(&mut rand::rng());
        create_outputs_statement(
            &[StealthOutputWitness {
                witness: OutputWitness {
                    amount,
                    minimum_value_promise,
                    mask,
                    sender_public_nonce: Default::default(),
                    encrypted_data: EncryptedData::try_from(vec![0; EncryptedData::min_size()]).unwrap(),
                    resource_view_key: None,
                },
                auth: SpendAuthorization::Key(RistrettoPublicKeyBytes::default()),
                tag: UtxoTag::new(0),
            }],
            Amount::zero(),
        )
        .unwrap()
    }

    #[test]
    fn it_is_valid_if_proof_is_valid() {
        let proof = create_valid_proof(100, 0);
        validate_stealth_outputs_statement(&proof, None).unwrap();
    }

    #[test]
    fn it_refuses_to_spend_one_input_twice() {
        let mask = RistrettoSecretKey::random(&mut rand::rng());
        let input = StealthInputWitness::new(MaskAndValue { mask, value: 100 });

        let err = create_transfer_statement(
            [input.clone(), input],
            Amount::zero(),
            &[] as &[StealthOutputWitness],
            Amount::from(200u64),
        )
        .unwrap_err();

        assert!(
            matches!(err, WalletCryptoError::InvalidArgument { name: "inputs", .. }),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn it_is_invalid_if_minimum_value_changed() {
        let mut proof = create_valid_proof(100, 100);
        proof.outputs[0].output.minimum_value_promise = 99;
        validate_stealth_outputs_statement(&proof, None).unwrap_err();
        proof.outputs[0].output.minimum_value_promise = 1000;
        validate_stealth_outputs_statement(&proof, None).unwrap_err();
    }
}
