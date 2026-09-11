//    Copyright 2025 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use std::collections::BTreeSet;

use log::*;
use ootle_byte_type::ConvertFromByteType;
use tari_crypto::{
    ristretto::{RistrettoPublicKey, RistrettoSecretKey, pedersen::PedersenCommitment},
    tari_utilities::ByteArrayError,
};
use tari_template_lib::types::{Amount, stealth::StealthTransferStatement};

use crate::{
    crypto::{commit_amount, messages, try_decode_to_signature},
    resource_container::ResourceError,
    stealth,
    stealth::ValidatedStealthOutput,
};

const LOG_TARGET: &str = "tari::engine_types::stealth::transfer";

#[derive(Debug, Clone)]
pub struct ValidatedStealthTransfer {
    pub outputs: Vec<ValidatedStealthOutput>,
    pub revealed_output_amount: Amount,
}

/// The native-execution metering points [`validate_transfer`] (plus the per-input spend
/// authorisation pass) costs for this statement. Charged against the payment-funded allowance
/// *before* any verification runs, so a non-paying transaction cannot extract the crypto work for
/// free. Must stay in lock-step with what [`validate_transfer`] actually verifies: a revealed-only
/// statement (no stealth inputs or outputs) verifies no balance proof and no range proof, so it
/// prices at zero, and the per-output ElGamal viewable-balance proof is only verified (and only
/// priced) when the resource carries a view key.
pub fn transfer_native_points(transfer: &StealthTransferStatement, has_view_key: bool) -> u64 {
    transfer_native_points_for_shape(
        transfer.inputs_statement.inputs.len(),
        transfer.outputs_statement.outputs.len(),
        has_view_key,
    )
}

/// [`transfer_native_points`] priced from a statement's shape alone, before the statement is built.
/// A builder must know the cost while choosing that shape — the range proofs it would have to
/// generate to produce a statement are the very work being priced — so the cost cannot depend on
/// anything but the input and output counts and the resource's view key.
pub fn transfer_native_points_for_shape(num_inputs: usize, num_outputs: usize, has_view_key: bool) -> u64 {
    use crate::limits::NativeExecutionPoints as P;
    if num_inputs == 0 && num_outputs == 0 {
        return 0;
    }
    let per_output = P::PER_OUTPUT.saturating_add(if has_view_key {
        P::PER_OUTPUT_VIEWABLE_SURCHARGE
    } else {
        0
    });
    P::PER_STATEMENT
        .saturating_add(per_output.saturating_mul(num_outputs as u64))
        .saturating_add(P::PER_INPUT.saturating_mul(num_inputs as u64))
}

pub fn validate_transfer(
    transfer: &StealthTransferStatement,
    view_key: Option<&RistrettoPublicKey>,
) -> Result<ValidatedStealthTransfer, ResourceError> {
    basic_validations(transfer)?;
    let validated_outputs = stealth::validate_stealth_outputs_statement(&transfer.outputs_statement, view_key)?;

    let balance_proof = transfer
        .balance_proof
        .as_ref()
        .map(|s| {
            try_decode_to_signature(s).ok_or_else(|| ResourceError::InvalidBalanceProof {
                details: "Malformed balance proof".to_string(),
            })
        })
        .transpose()?;

    // EDGE CASE: If there are no inputs and no outputs, the public excess will be 0.G
    if transfer.inputs_statement.inputs.is_empty() && validated_outputs.is_empty() {
        // Ensure that the range proof is empty
        if !transfer.outputs_statement.agg_range_proof.is_empty() {
            return Err(ResourceError::InvalidBalanceProof {
                details: "Range proof must be empty when there are no inputs or outputs".to_string(),
            });
        }

        // In this case, the public excess is 0 (s = r + e.0 = r) which leaks the secret nonce. Probably fine, but
        // instead, we enforce that a None signature MUST be used for this case.
        if balance_proof.is_some() {
            return Err(ResourceError::InvalidBalanceProof {
                details: "Balance proof signature verification failed for revealed amount. This indicates that the \
                          transfer statement provided a balance proof when there are no stealth inputs or outputs, \
                          None is required."
                    .to_string(),
            });
        }

        return Ok(ValidatedStealthTransfer {
            outputs: vec![],
            revealed_output_amount: transfer.outputs_statement.revealed_output_amount,
        });
    }
    let balance_proof = balance_proof.ok_or_else(|| ResourceError::InvalidBalanceProof {
        details: "Balance proof must be provided when there are stealth inputs or outputs".to_string(),
    })?;

    let agg_outputs = validated_outputs.iter().fold(RistrettoPublicKey::default(), |acc, o| {
        acc + o.output.commitment.as_public_key()
    });

    let agg_inputs = transfer
        .inputs_statement
        .inputs
        .iter()
        .try_fold(RistrettoPublicKey::default(), |sum, input| {
            let commit = PedersenCommitment::convert_from_byte_type(&input.commitment)?;
            Ok::<_, ByteArrayError>(sum + commit.as_public_key())
        })
        .map_err(|e| ResourceError::InvalidConfidentialProof {
            details: format!("Malformed commitment in transfer inputs: {e}"),
        })?;

    // We assume that the input amount is available and only check that the maths is correct. The engine is responsible
    // for checking that the input amount is actually available.
    let revealed_input_commit = commit_amount(
        &RistrettoSecretKey::default(),
        transfer.inputs_statement.revealed_amount,
    )
    .ok_or_else(|| ResourceError::InvalidBalanceProof {
        details: "Revealed input amount must be non-negative".to_string(),
    })?;
    let revealed_output_commit = commit_amount(
        &RistrettoSecretKey::default(),
        transfer.outputs_statement.revealed_output_amount,
    )
    .ok_or_else(|| ResourceError::InvalidBalanceProof {
        details: "Revealed output amount must be non-negative".to_string(),
    })?;

    let public_excess =
        agg_inputs + revealed_input_commit.as_public_key() - &agg_outputs - revealed_output_commit.as_public_key();

    debug!(
        target: LOG_TARGET,
        "Validating transfer: revealed input amount: {}, revealed output amount: {}, public excess: {}, nonce: {}",
        transfer.inputs_statement.revealed_amount,
        transfer.outputs_statement.revealed_output_amount,
        public_excess,
        balance_proof.get_public_nonce()
    );

    let message = messages::stealth_balance_proof64(
        &public_excess,
        balance_proof.get_public_nonce(),
        &transfer.inputs_statement,
        &transfer.outputs_statement,
    );

    if !balance_proof.verify_raw_uniform(&public_excess, &message) {
        return Err(ResourceError::InvalidBalanceProof {
            details: "Balance proof signature verification failed. This typically indicates that the transfer \
                      statement total input amount != total output amount."
                .to_string(),
        });
    }

    Ok(ValidatedStealthTransfer {
        outputs: validated_outputs,
        revealed_output_amount: transfer.outputs_statement.revealed_output_amount,
    })
}

fn basic_validations(transfer: &StealthTransferStatement) -> Result<(), ResourceError> {
    // The excess folds the inputs positionally, so a commitment listed n times contributes n times its value to a
    // balance proof its spender can still construct, while the spend downs the one UTXO once.
    let mut seen = BTreeSet::new();
    for input in &transfer.inputs_statement.inputs {
        if !seen.insert(input.commitment) {
            return Err(ResourceError::InvalidSpend {
                details: format!("Duplicate input commitment {} in transfer statement", input.commitment),
            });
        }
    }

    if transfer.inputs_statement.revealed_amount.is_negative() {
        return Err(ResourceError::InvalidBalanceProof {
            details: format!(
                "Revealed input amount must be non-negative: {}",
                transfer.inputs_statement.revealed_amount
            ),
        });
    }
    if transfer.outputs_statement.revealed_output_amount.is_negative() {
        return Err(ResourceError::InvalidBalanceProof {
            details: format!(
                "Revealed output amount must be non-negative: {}",
                transfer.outputs_statement.revealed_output_amount
            ),
        });
    }

    if transfer.inputs_statement.revealed_amount.is_zero() && transfer.inputs_statement.inputs.is_empty() {
        return Err(ResourceError::InvalidBalanceProof {
            details: "No inputs or revealed inputs provided".to_string(),
        });
    }

    // Check the balance if there are no stealth inputs or outputs. Since the excess will be zero in this case, the
    // balance signature (r + 0.e) does not prove the balance.
    if transfer.inputs_statement.inputs.is_empty() && transfer.outputs_statement.outputs.is_empty() {
        if transfer.inputs_statement.revealed_amount != transfer.outputs_statement.revealed_output_amount {
            return Err(ResourceError::InvalidBalanceProof {
                details: format!(
                    "Revealed input amount {} does not match revealed output amount {} - no stealth inputs or outputs \
                     provided",
                    transfer.inputs_statement.revealed_amount, transfer.outputs_statement.revealed_output_amount
                ),
            });
        }
    } else if transfer
        .balance_proof
        .is_none_or(|p| p.public_nonce().is_zero() || p.signature().is_zero())
    {
        return Err(ResourceError::InvalidBalanceProof {
            details: "Balance proof must be provided and public nonce and signature cannot be zero".to_string(),
        });
    } else {
        // Ok
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::NativeExecutionPoints as P;

    #[test]
    fn empty_statement_prices_at_zero() {
        // A revealed-only statement verifies no balance proof and no range proof, matching the
        // short-circuit in `validate_transfer`.
        assert_eq!(transfer_native_points_for_shape(0, 0, false), 0);
        assert_eq!(transfer_native_points_for_shape(0, 0, true), 0);
    }

    #[test]
    fn price_is_statement_plus_per_output_plus_per_input() {
        assert_eq!(
            transfer_native_points_for_shape(3, 2, false),
            P::PER_STATEMENT + 2 * P::PER_OUTPUT + 3 * P::PER_INPUT
        );
    }

    #[test]
    fn view_key_adds_the_surcharge_per_output() {
        let no_vk = transfer_native_points_for_shape(1, 2, false);
        let with_vk = transfer_native_points_for_shape(1, 2, true);
        assert_eq!(with_vk - no_vk, 2 * P::PER_OUTPUT_VIEWABLE_SURCHARGE);
    }
}
