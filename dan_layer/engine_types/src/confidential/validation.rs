//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::iter;

use tari_common_types::types::{Commitment, PublicKey};
use tari_crypto::{
    extended_range_proof::{ExtendedRangeProofService, Statement},
    ristretto::bulletproofs_plus::RistrettoAggregatedPublicStatement,
};
use tari_template_lib::models::ConfidentialOutputProof;
use tari_utilities::ByteArray;

use super::get_range_proof_service;
use crate::{confidential::ConfidentialOutput, resource_container::ResourceError};

#[derive(Debug)]
pub struct ValidatedConfidentialProof {
    pub output: ConfidentialOutput,
    pub change_output: Option<ConfidentialOutput>,
}

pub fn validate_confidential_proof(
    proof: &ConfidentialOutputProof,
) -> Result<ValidatedConfidentialProof, ResourceError> {
    if proof.revealed_amount.is_negative() {
        return Err(ResourceError::InvalidConfidentialProof {
            details: "Revealed amount must be positive".to_string(),
        });
    }

    let output_commitment = Commitment::from_bytes(&proof.output_statement.commitment).map_err(|_| {
        ResourceError::InvalidConfidentialProof {
            details: "Invalid commitment".to_string(),
        }
    })?;

    let output_public_nonce =
        PublicKey::from_bytes(proof.output_statement.sender_public_nonce.as_bytes()).map_err(|_| {
            ResourceError::InvalidConfidentialProof {
                details: "Invalid sender public nonce".to_string(),
            }
        })?;

    let change = proof
        .change_statement
        .as_ref()
        .map(|stmt| {
            let commitment =
                Commitment::from_bytes(&stmt.commitment).map_err(|_| ResourceError::InvalidConfidentialProof {
                    details: "Invalid commitment".to_string(),
                })?;
            let stealth_public_nonce = PublicKey::from_bytes(stmt.sender_public_nonce.as_bytes()).map_err(|_| {
                ResourceError::InvalidConfidentialProof {
                    details: "Invalid sender public nonce".to_string(),
                }
            })?;

            Ok(ConfidentialOutput {
                commitment,
                stealth_public_nonce: Some(stealth_public_nonce),
                encrypted_value: Some(stmt.encrypted_value),
                minimum_value_promise: stmt.minimum_value_promise,
            })
        })
        .transpose()?;

    validate_bullet_proof(proof)?;

    Ok(ValidatedConfidentialProof {
        output: ConfidentialOutput {
            commitment: output_commitment,
            stealth_public_nonce: Some(output_public_nonce),
            encrypted_value: Some(proof.output_statement.encrypted_value),
            minimum_value_promise: proof.output_statement.minimum_value_promise,
        },
        change_output: change,
    })
}

fn validate_bullet_proof(proof: &ConfidentialOutputProof) -> Result<(), ResourceError> {
    let statements = iter::once(&proof.output_statement)
        .chain(proof.change_statement.as_ref())
        .cloned()
        .map(|stmt| {
            let commitment =
                Commitment::from_bytes(&stmt.commitment).map_err(|_| ResourceError::InvalidConfidentialProof {
                    details: "Invalid commitment".to_string(),
                })?;
            Ok(Statement {
                commitment,
                minimum_value_promise: stmt.minimum_value_promise,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    let agg_factor = statements.len();
    let public_statement = RistrettoAggregatedPublicStatement::init(statements).unwrap();

    let proofs = vec![&proof.range_proof];
    get_range_proof_service(agg_factor)
        .verify_batch(proofs, vec![&public_statement])
        .map_err(|e| ResourceError::InvalidConfidentialProof {
            details: format!("Invalid range proof: {}", e),
        })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use rand::rngs::OsRng;
    use tari_common_types::types::PrivateKey;
    use tari_crypto::keys::SecretKey;
    use tari_template_lib::models::Amount;

    use super::*;
    use crate::confidential::{generate_confidential_proof, proof::ConfidentialProofStatement};

    mod validate_confidential_proof {
        use super::*;

        fn create_valid_proof(amount: Amount, minimum_value_promise: u64) -> ConfidentialOutputProof {
            let mask = PrivateKey::random(&mut OsRng);
            generate_confidential_proof(
                ConfidentialProofStatement {
                    amount,
                    minimum_value_promise,
                    mask,
                    sender_public_nonce: Default::default(),
                },
                None,
            )
            .unwrap()
        }

        #[test]
        fn it_is_valid_if_proof_is_valid() {
            let proof = create_valid_proof(100.into(), 0);
            validate_confidential_proof(&proof).unwrap();
        }

        #[test]
        fn it_is_invalid_if_minimum_value_changed() {
            let mut proof = create_valid_proof(100.into(), 100);
            proof.output_statement.minimum_value_promise = 99;
            validate_confidential_proof(&proof).unwrap_err();
            proof.output_statement.minimum_value_promise = 1000;
            validate_confidential_proof(&proof).unwrap_err();
        }
    }
}
