//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_common_types::types::{Commitment, PrivateKey, PublicKey};
use tari_crypto::{
    commitment::HomomorphicCommitmentFactory,
    extended_range_proof::{ExtendedRangeProofService, Statement},
    keys::{PublicKey as _, SecretKey},
    ristretto::{bulletproofs_plus::RistrettoAggregatedPublicStatement, RistrettoSecretKey},
    tari_utilities::ByteArray,
};
use tari_template_lib::models::{Amount, ConfidentialOutputStatement, ViewableBalanceProof};

use super::{challenges, get_commitment_factory, get_range_proof_service};
use crate::{
    confidential::{elgamal::ElgamalVerifiableBalance, ConfidentialOutput},
    resource_container::ResourceError,
};

#[derive(Debug)]
pub struct ValidatedConfidentialProof {
    pub output: Option<ConfidentialOutput>,
    pub change_output: Option<ConfidentialOutput>,
    pub output_revealed_amount: Amount,
    pub change_revealed_amount: Amount,
}

pub fn validate_confidential_proof(
    proof: &ConfidentialOutputStatement,
    view_key: Option<&PublicKey>,
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
            let output_commitment =
                Commitment::from_canonical_bytes(statement.commitment.as_bytes()).map_err(|_| {
                    ResourceError::InvalidConfidentialProof {
                        details: "Invalid commitment".to_string(),
                    }
                })?;

            let output_public_nonce = PublicKey::from_canonical_bytes(statement.sender_public_nonce.as_bytes())
                .map_err(|_| ResourceError::InvalidConfidentialProof {
                    details: "Invalid sender public nonce".to_string(),
                })?;

            let viewable_balance = validate_elgamal_verifiable_balance_proof(
                &output_commitment,
                view_key,
                statement.viewable_balance_proof.as_ref(),
            )?;

            Ok(ConfidentialOutput {
                commitment: output_commitment,
                stealth_public_nonce: output_public_nonce,
                encrypted_data: statement.encrypted_data.clone(),
                minimum_value_promise: statement.minimum_value_promise,
                viewable_balance,
            })
        })
        .transpose()?;

    let maybe_change = proof
        .change_statement
        .as_ref()
        .map(|stmt| {
            let commitment = Commitment::from_canonical_bytes(&*stmt.commitment).map_err(|_| {
                ResourceError::InvalidConfidentialProof {
                    details: "Invalid commitment".to_string(),
                }
            })?;

            let stealth_public_nonce = PublicKey::from_canonical_bytes(&*stmt.sender_public_nonce).map_err(|_| {
                ResourceError::InvalidConfidentialProof {
                    details: "Invalid sender public nonce".to_string(),
                }
            })?;

            let viewable_balance =
                validate_elgamal_verifiable_balance_proof(&commitment, view_key, stmt.viewable_balance_proof.as_ref())?;

            Ok(ConfidentialOutput {
                commitment,
                stealth_public_nonce,
                encrypted_data: stmt.encrypted_data.clone(),
                minimum_value_promise: stmt.minimum_value_promise,
                viewable_balance,
            })
        })
        .transpose()?;

    if maybe_output.is_none() && maybe_change.is_none() {
        if !proof.range_proof.is_empty() {
            return Err(ResourceError::InvalidConfidentialProof {
                details: "Range proof is invalid because it was provided (non-empty) but the proof contained no \
                          confidential outputs"
                    .to_string(),
            });
        }
    } else {
        validate_bullet_proof(proof)?;
    }

    Ok(ValidatedConfidentialProof {
        output: maybe_output,
        change_output: maybe_change,
        output_revealed_amount: proof.output_revealed_amount,
        change_revealed_amount: proof.change_revealed_amount,
    })
}

pub fn validate_elgamal_verifiable_balance_proof(
    commitment: &Commitment,
    view_key: Option<&PublicKey>,
    viewable_balance_proof: Option<&ViewableBalanceProof>,
) -> Result<Option<ElgamalVerifiableBalance>, ResourceError> {
    // Check that if a view key is provided, then a viewable balance proof is also provided and vice versa
    let Some(view_key) = view_key else {
        if viewable_balance_proof.is_none() {
            return Ok(None);
        }
        return Err(ResourceError::InvalidConfidentialProof {
            details: "ViewableBalanceProof provided for a resource that is not viewable".to_string(),
        });
    };

    let Some(proof) = viewable_balance_proof else {
        return Err(ResourceError::InvalidConfidentialProof {
            details: "ViewableBalanceProof is required for a viewable resource".to_string(),
        });
    };

    // Decode and check that each field is well-formed
    let encrypted = PublicKey::from_canonical_bytes(&*proof.elgamal_encrypted).map_err(|_| {
        ResourceError::InvalidConfidentialProof {
            details: "Invalid value for E".to_string(),
        }
    })?;

    let elgamal_public_nonce = PublicKey::from_canonical_bytes(&*proof.elgamal_public_nonce).map_err(|_| {
        ResourceError::InvalidConfidentialProof {
            details: "Invalid public key for R".to_string(),
        }
    })?;

    let c_prime =
        Commitment::from_canonical_bytes(&*proof.c_prime).map_err(|_| ResourceError::InvalidConfidentialProof {
            details: "Invalid commitment for C'".to_string(),
        })?;

    let e_prime =
        Commitment::from_canonical_bytes(&*proof.e_prime).map_err(|_| ResourceError::InvalidConfidentialProof {
            details: "Invalid commitment for E'".to_string(),
        })?;

    let r_prime =
        PublicKey::from_canonical_bytes(&*proof.r_prime).map_err(|_| ResourceError::InvalidConfidentialProof {
            details: "Invalid public key for R'".to_string(),
        })?;

    let s_v = PrivateKey::from_canonical_bytes(&*proof.s_v).map_err(|_| ResourceError::InvalidConfidentialProof {
        details: "Invalid private key for s_v".to_string(),
    })?;

    let s_m = PrivateKey::from_canonical_bytes(&*proof.s_m).map_err(|_| ResourceError::InvalidConfidentialProof {
        details: "Invalid private key for s_m".to_string(),
    })?;

    let s_r = &PrivateKey::from_canonical_bytes(&*proof.s_r).map_err(|_| ResourceError::InvalidConfidentialProof {
        details: "Invalid private key for s_r".to_string(),
    })?;

    // Fiat-Shamir challenge
    let e = &RistrettoSecretKey::from_uniform_bytes(&challenges::viewable_balance_proof_challenge64(
        commitment,
        view_key,
        proof.as_challenge_fields(),
    ))
    // TODO: it would be better if from_uniform_bytes took a [u8; 64]
    .expect("INVARIANT VIOLATION: RistrettoSecretKey::from_uniform_bytes and hash output length mismatch");

    // Check eC + C' ?= s_m.G + sv.H
    let left = e * commitment.as_public_key() + c_prime.as_public_key();
    let right = get_commitment_factory().commit(&s_m, &s_v);
    if left != *right.as_public_key() {
        return Err(ResourceError::InvalidConfidentialProof {
            details: "Invalid viewable balance proof (eC + C' != s_m.G + s_v.H)".to_string(),
        });
    }

    // Check eE + E' ?= s_v.G + s_r.P
    let left = e * &encrypted + e_prime.as_public_key();
    let right = PublicKey::from_secret_key(&s_v) + s_r * view_key;
    if left != right {
        return Err(ResourceError::InvalidConfidentialProof {
            details: "Invalid viewable balance proof (eE + E' != s_v.G + s_r.P)".to_string(),
        });
    }

    // Check eR + R' ?= s_r.G
    let left = e * &elgamal_public_nonce + r_prime;
    let right = PublicKey::from_secret_key(s_r);
    if left != right {
        return Err(ResourceError::InvalidConfidentialProof {
            details: "Invalid viewable balance proof (eR + R' != s_r.G)".to_string(),
        });
    }

    Ok(Some(ElgamalVerifiableBalance {
        encrypted,
        public_nonce: elgamal_public_nonce,
    }))
}

fn validate_bullet_proof(proof: &ConfidentialOutputStatement) -> Result<(), ResourceError> {
    let statements = proof
        .output_statement
        .iter()
        .chain(proof.change_statement.iter())
        .map(|stmt| {
            let commitment = Commitment::from_canonical_bytes(&*stmt.commitment).map_err(|_| {
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
