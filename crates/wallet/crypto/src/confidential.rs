//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use ootle_byte_type::ToByteType;
use tari_crypto::{ristretto::RistrettoSecretKey, tari_utilities::ByteArray};
use tari_engine_types::crypto::commit_u64_amount;
use tari_template_lib_types::{
    Amount,
    confidential::{ConfidentialOutputStatement, ConfidentialWithdrawProof},
    crypto::RistrettoPublicKeyBytes,
    stealth::UnspentOutput,
};

use crate::{
    MaskAndValue,
    OutputWitness,
    WalletCryptoError,
    balance_proof::generate_confidential_balance_proof,
    bullet_proof::generate_extended_bullet_proof,
    error::StealthProofError,
    viewable_balance_proof::generate_elgamal_viewable_balance_proof,
};

pub fn create_withdraw_proof(
    inputs: &[MaskAndValue],
    input_revealed_amount: Amount,
    output_statement: Option<&OutputWitness>,
    output_revealed_amount: Amount,
    change_statement: Option<&OutputWitness>,
    change_revealed_amount: Amount,
) -> Result<ConfidentialWithdrawProof, WalletCryptoError> {
    check_withdraw_balance(
        inputs,
        input_revealed_amount,
        output_statement,
        output_revealed_amount,
        change_statement,
        change_revealed_amount,
    )?;

    let output_proof = create_output_statement(
        output_statement,
        output_revealed_amount,
        change_statement,
        change_revealed_amount,
    )?;
    let (input_commitments, agg_input_mask) = inputs.iter().try_fold(
        (Vec::with_capacity(inputs.len()), RistrettoSecretKey::default()),
        |(mut commitments, agg_input), input| {
            let commitment = commit_u64_amount(&input.mask, input.value);
            commitments.push(commitment.to_byte_type());
            Ok::<_, WalletCryptoError>((commitments, agg_input + &input.mask))
        },
    )?;

    let output_revealed_amount = output_proof.output_revealed_amount + output_proof.change_revealed_amount;
    let balance_proof = generate_confidential_balance_proof(
        &agg_input_mask,
        &input_revealed_amount,
        output_statement.as_ref().map(|o| &o.mask),
        change_statement.as_ref().map(|ch| &ch.mask),
        &output_revealed_amount,
    );

    let output_statement = output_proof.output;
    let change_statement = output_proof.change_statement;

    Ok(ConfidentialWithdrawProof {
        inputs: input_commitments,
        input_revealed_amount,
        output_proof: ConfidentialOutputStatement {
            output: output_statement,
            change_statement,
            range_proof: output_proof.range_proof,
            output_revealed_amount: output_proof.output_revealed_amount,
            change_revealed_amount: output_proof.change_revealed_amount,
        },
        balance_proof,
    })
}

/// The balance proof signs with the blinding factors alone, so it only verifies when the committed values on each
/// side are equal. An unbalanced withdraw yields a proof the engine is guaranteed to reject.
fn check_withdraw_balance(
    inputs: &[MaskAndValue],
    input_revealed_amount: Amount,
    output_statement: Option<&OutputWitness>,
    output_revealed_amount: Amount,
    change_statement: Option<&OutputWitness>,
    change_revealed_amount: Amount,
) -> Result<(), WalletCryptoError> {
    let overflow = || WalletCryptoError::InvalidArgument {
        name: "amounts",
        details: "withdraw amounts overflow".to_string(),
    };
    let total_in = inputs
        .iter()
        .map(|input| Amount::from(input.value))
        .try_fold(input_revealed_amount, |acc, v| acc.checked_add(v))
        .ok_or_else(overflow)?;
    let total_out = output_statement
        .into_iter()
        .chain(change_statement)
        .map(|stmt| Amount::from(stmt.amount))
        .chain([output_revealed_amount, change_revealed_amount])
        .try_fold(Amount::zero(), |acc, v| acc.checked_add(v))
        .ok_or_else(overflow)?;

    if total_in != total_out {
        return Err(WalletCryptoError::UnbalancedWithdraw {
            inputs: total_in,
            outputs: total_out,
        });
    }
    Ok(())
}

pub fn create_output_statement(
    output_statement: Option<&OutputWitness>,
    output_revealed_amount: Amount,
    change_statement: Option<&OutputWitness>,
    change_revealed_amount: Amount,
) -> Result<ConfidentialOutputStatement, StealthProofError> {
    let proof_change_statement = change_statement
        .as_ref()
        .map(|stmt| -> Result<_, StealthProofError> {
            let change_commitment = stmt.to_commitment();
            Ok(UnspentOutput {
                commitment: change_commitment.to_byte_type(),
                sender_public_nonce: RistrettoPublicKeyBytes::from_bytes(stmt.sender_public_nonce.as_bytes())
                    .expect("[generate_confidential_proof] change nonce"),
                encrypted_data: stmt.encrypted_data.clone(),
                minimum_value_promise: stmt.minimum_value_promise,
                viewable_balance_proof: stmt
                    .resource_view_key
                    .as_ref()
                    .map(|view_key| {
                        generate_elgamal_viewable_balance_proof(&stmt.mask, stmt.amount, &change_commitment, view_key)
                    })
                    .transpose()?,
            })
        })
        .transpose()?;
    let confidential_output_value = output_statement.as_ref().map(|o| o.amount).unwrap_or_default();

    let proof_output_statement = output_statement
        .as_ref()
        .map(|stmt| {
            let commitment = stmt.to_commitment();
            Ok::<_, StealthProofError>(UnspentOutput {
                commitment: commitment.to_byte_type(),
                sender_public_nonce: stmt.sender_public_nonce.to_byte_type(),
                encrypted_data: stmt.encrypted_data.clone(),
                minimum_value_promise: stmt.minimum_value_promise,
                viewable_balance_proof: stmt
                    .resource_view_key
                    .as_ref()
                    .map(|view_key| {
                        generate_elgamal_viewable_balance_proof(
                            &stmt.mask,
                            confidential_output_value,
                            &commitment,
                            view_key,
                        )
                    })
                    .transpose()?,
            })
        })
        .transpose()?;

    let output_range_proof = generate_extended_bullet_proof(output_statement.into_iter().chain(change_statement))?;

    Ok(ConfidentialOutputStatement {
        output: proof_output_statement,
        change_statement: proof_change_statement,
        range_proof: output_range_proof,
        output_revealed_amount,
        change_revealed_amount,
    })
}

#[cfg(test)]
mod tests {
    use tari_crypto::{keys::SecretKey, ristretto::RistrettoSecretKey};
    use tari_engine_types::confidential::validate_confidential_statement;
    use tari_template_lib_types::EncryptedData;

    use super::*;

    fn witness(amount: u64, minimum_value_promise: u64) -> OutputWitness {
        OutputWitness {
            amount,
            minimum_value_promise,
            mask: RistrettoSecretKey::random(&mut rand::rng()),
            sender_public_nonce: Default::default(),
            encrypted_data: EncryptedData::try_from(vec![0; EncryptedData::min_size()]).unwrap(),
            resource_view_key: None,
        }
    }

    fn create_valid_proof(amount: u64, minimum_value_promise: u64) -> ConfidentialOutputStatement {
        create_output_statement(
            Some(&witness(amount, minimum_value_promise)),
            Default::default(),
            None,
            Default::default(),
        )
        .unwrap()
    }

    #[test]
    fn it_is_valid_if_proof_is_valid() {
        let proof = create_valid_proof(100, 1);
        validate_confidential_statement(&proof, None).unwrap();
    }

    /// A range proof with a zero minimum value promise also proves a commitment to zero, so it cannot show the output
    /// holds any of the resource.
    #[test]
    fn it_is_invalid_if_an_output_promises_no_minimum_value() {
        let zero_change = create_output_statement(
            Some(&witness(100, 1)),
            Default::default(),
            Some(&witness(0, 0)),
            Default::default(),
        )
        .unwrap();

        for proof in [create_valid_proof(100, 0), zero_change] {
            let err = validate_confidential_statement(&proof, None).unwrap_err();
            assert!(
                err.to_string().contains("must promise a minimum value of at least 1"),
                "unexpected error: {err}"
            );
        }
    }

    #[test]
    fn it_cannot_prove_a_zero_value_output_with_a_minimum_value_promise() {
        create_output_statement(Some(&witness(0, 1)), Default::default(), None, Default::default()).unwrap_err();
    }

    #[test]
    fn it_is_invalid_if_minimum_value_changed() {
        let mut proof = create_valid_proof(100, 100);
        proof.output.as_mut().unwrap().minimum_value_promise = 99;
        validate_confidential_statement(&proof, None).unwrap_err();
        proof.output.as_mut().unwrap().minimum_value_promise = 1000;
        validate_confidential_statement(&proof, None).unwrap_err();
    }
}
