//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::iter;

use tari_common_types::types::{BulletRangeProof, Commitment};
use tari_crypto::{
    extended_range_proof::{ExtendedRangeProofService, Statement},
    ristretto::bulletproofs_plus::RistrettoAggregatedPublicStatement,
};
use tari_template_lib::models::{Amount, ConfidentialProof};
use tari_utilities::ByteArray;

use crate::{crypto, resource_container::ResourceError};

#[derive(Debug, Clone)]
pub struct ValidatedConfidentialWithdrawProof {
    pub output_commitment: Commitment,
    pub output_minimum_value_promise: u64,
    pub change_commitment: Option<Commitment>,
    pub change_minimum_value_promise: Option<u64>,
    pub range_proof: BulletRangeProof,
    pub revealed_amount: Amount,
}

pub fn validate_confidential_proof(
    proof: &ConfidentialProof,
) -> Result<(Commitment, Option<Commitment>), ResourceError> {
    if proof.revealed_amount.is_negative() {
        return Err(ResourceError::InvalidConfidentialProof {
            details: "Revealed amount must be positive".to_string(),
        });
    }
    validate_bullet_proof(proof)?;

    let output_commitment = Commitment::from_bytes(&proof.output_statement.commitment).map_err(|_| {
        ResourceError::InvalidConfidentialProof {
            details: "Invalid commitment".to_string(),
        }
    })?;

    let change_commitment = proof
        .change_statement
        .as_ref()
        .map(|stmt| {
            Commitment::from_bytes(&stmt.commitment).map_err(|_| ResourceError::InvalidConfidentialProof {
                details: "Invalid commitment".to_string(),
            })
        })
        .transpose()?;

    Ok((output_commitment, change_commitment))
}

fn validate_bullet_proof(proof: &ConfidentialProof) -> Result<(), ResourceError> {
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
    crypto::range_proof_service(agg_factor)
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
    use crate::crypto::{generate_confidential_proof, ConfidentialProofStatement};

    mod validate_confidential_proof {
        use super::*;

        fn create_valid_proof(amount: Amount, minimum_value_promise: u64) -> ConfidentialProof {
            let mask = PrivateKey::random(&mut OsRng);
            generate_confidential_proof(
                ConfidentialProofStatement {
                    amount,
                    minimum_value_promise,
                    mask,
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
