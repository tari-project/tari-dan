//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_common_types::types::{Commitment, PublicKey};
use tari_crypto::{
    extended_range_proof::{ExtendedRangeProofService, Statement},
    ristretto::bulletproofs_plus::RistrettoAggregatedPublicStatement,
    tari_utilities::ByteArray,
};
use tari_template_lib::models::{Amount, ConfidentialOutputProof};

use super::get_range_proof_service;
use crate::{confidential::ConfidentialOutput, resource_container::ResourceError};

#[derive(Debug)]
pub struct ValidatedConfidentialProof {
    pub output: Option<ConfidentialOutput>,
    pub change_output: Option<ConfidentialOutput>,
    pub output_revealed_amount: Amount,
    pub change_revealed_amount: Amount,
}

pub fn validate_confidential_proof(
    proof: &ConfidentialOutputProof,
) -> Result<ValidatedConfidentialProof, ResourceError> {
    if proof.output_revealed_amount.is_negative() || proof.change_revealed_amount.is_negative() {
        return Err(ResourceError::InvalidConfidentialProof {
            details: "Revealed amounts must be positive".to_string(),
        });
    }

    let maybe_output = proof
        .output_statement
        .as_ref()
        .map(|statement| {
            let output_commitment = Commitment::from_canonical_bytes(&statement.commitment).map_err(|_| {
                ResourceError::InvalidConfidentialProof {
                    details: "Invalid commitment".to_string(),
                }
            })?;

            let output_public_nonce = PublicKey::from_canonical_bytes(statement.sender_public_nonce.as_bytes())
                .map_err(|_| ResourceError::InvalidConfidentialProof {
                    details: "Invalid sender public nonce".to_string(),
                })?;

            Ok(ConfidentialOutput {
                commitment: output_commitment,
                stealth_public_nonce: output_public_nonce,
                encrypted_data: statement.encrypted_data.clone(),
                minimum_value_promise: statement.minimum_value_promise,
            })
        })
        .transpose()?;

    let change =
        proof
            .change_statement
            .as_ref()
            .map(|stmt| {
                let commitment = Commitment::from_canonical_bytes(&stmt.commitment).map_err(|_| {
                    ResourceError::InvalidConfidentialProof {
                        details: "Invalid commitment".to_string(),
                    }
                })?;

                let stealth_public_nonce = PublicKey::from_canonical_bytes(stmt.sender_public_nonce.as_bytes())
                    .map_err(|_| ResourceError::InvalidConfidentialProof {
                        details: "Invalid sender public nonce".to_string(),
                    })?;

                Ok(ConfidentialOutput {
                    commitment,
                    stealth_public_nonce,
                    encrypted_data: stmt.encrypted_data.clone(),
                    minimum_value_promise: stmt.minimum_value_promise,
                })
            })
            .transpose()?;

    validate_bullet_proof(proof)?;

    Ok(ValidatedConfidentialProof {
        output: maybe_output,
        change_output: change,
        output_revealed_amount: proof.output_revealed_amount,
        change_revealed_amount: proof.change_revealed_amount,
    })
}

fn validate_bullet_proof(proof: &ConfidentialOutputProof) -> Result<(), ResourceError> {
    let statements = proof
        .output_statement
        .iter()
        .chain(proof.change_statement.iter())
        .map(|stmt| {
            let commitment = Commitment::from_canonical_bytes(&stmt.commitment).map_err(|_| {
                ResourceError::InvalidConfidentialProof {
                    details: "Invalid commitment".to_string(),
                }
            })?;
            Ok(Statement {
                commitment,
                minimum_value_promise: stmt.minimum_value_promise,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    // Either 0, 1 or 2
    let agg_factor = statements.len();
    if agg_factor == 0 {
        // No outputs, so no rangeproof needed (revealed mint)
        if proof.range_proof.is_empty() {
            return Ok(());
        }
        return Err(ResourceError::InvalidConfidentialProof {
            details: "Range proof is invalid because it was provided but the proof contained no outputs".to_string(),
        });
    }

    let public_statement = RistrettoAggregatedPublicStatement::init(statements).unwrap();

    let proofs = vec![&proof.range_proof];
    get_range_proof_service(agg_factor)
        .verify_batch(proofs, vec![&public_statement])
        .map_err(|e| ResourceError::InvalidConfidentialProof {
            details: format!("Invalid range proof: {}", e),
        })?;

    Ok(())
}
