//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_common_types::types::{BulletRangeProof, Commitment, PrivateKey, PublicKey};
use tari_crypto::{
    commitment::HomomorphicCommitment,
    extended_range_proof::{ExtendedRangeProofService, Statement},
    keys::{PublicKey as _, SecretKey},
    ristretto::{
        bulletproofs_plus::RistrettoAggregatedPublicStatement,
        pedersen::commitment_factory::PedersenCommitmentFactory,
    },
    signatures::CommitmentSignature,
};
use tari_template_lib::models::ConfidentialProof;
use tari_utilities::ByteArray;

use crate::{crypto, runtime::RuntimeError};

pub fn validate_confidential_proof(proof: ConfidentialProof) -> Result<(PublicKey, BulletRangeProof), RuntimeError> {
    let commitment = Commitment::from_bytes(&proof.commitment).map_err(|_| RuntimeError::InvalidConfidentialProof {
        details: "Invalid commitment".to_string(),
    })?;
    let public_mask =
        PublicKey::from_bytes(proof.public_mask.as_bytes()).map_err(|_| RuntimeError::InvalidConfidentialProof {
            details: "Invalid public mask".to_string(),
        })?;

    let signature = extract_commitment_signature(&proof.knowledge_proof)?;
    // TODO: use extended commitments, what's missing is a way to sign for a 4-degree commitment (mask, value, asset
    // tag, asset instance)
    let factory = PedersenCommitmentFactory::default();
    let challenge = crypto::challenges::confidential_commitment_proof(&commitment);
    let challenge =
        crypto::challenges::strong_fiat_shamir(&public_mask, signature.public_nonce().as_public_key(), &challenge);

    let challenge = PrivateKey::from_bytes(&challenge).expect("Hash to 32-byte scalar failed");

    if !signature.verify(&commitment, &challenge, &factory) {
        return Err(RuntimeError::InvalidConfidentialProof {
            details: "Invalid proof of knowledge signature".to_string(),
        });
    }

    validate_bullet_proof(&proof, &commitment)?;

    Ok((commitment.as_public_key().clone(), BulletRangeProof(proof.range_proof)))
}

fn validate_bullet_proof(proof: &ConfidentialProof, commitment: &Commitment) -> Result<(), RuntimeError> {
    let statement = RistrettoAggregatedPublicStatement {
        statements: vec![Statement {
            commitment: commitment.clone(),
            minimum_value_promise: proof.minimum_value_promise,
        }],
    };

    crypto::range_proof_service()
        .verify_batch(vec![&proof.range_proof], vec![&statement])
        .map_err(|e| RuntimeError::InvalidConfidentialProof {
            details: format!("Invalid range proof: {}", e),
        })?;

    Ok(())
}

fn extract_commitment_signature(sig_bytes: &[u8]) -> Result<CommitmentSignature<PublicKey, PrivateKey>, RuntimeError> {
    if sig_bytes.len() != PublicKey::key_length() + PrivateKey::key_length() * 2 {
        return Err(RuntimeError::InvalidConfidentialProof {
            details: format!("Knowledge proof invalid length: {}", sig_bytes.len()),
        });
    }
    let public_nonce =
        HomomorphicCommitment::from_bytes(&sig_bytes[..32]).map_err(|_| RuntimeError::InvalidConfidentialProof {
            details: "Invalid public nonce in knowledge proof".to_string(),
        })?;

    let u = PrivateKey::from_bytes(&sig_bytes[32..64]).map_err(|_| RuntimeError::InvalidConfidentialProof {
        details: "Invalid u in knowledge proof".to_string(),
    })?;

    let v = PrivateKey::from_bytes(&sig_bytes[64..96]).map_err(|_| RuntimeError::InvalidConfidentialProof {
        details: "Invalid v in knowledge proof".to_string(),
    })?;

    Ok(CommitmentSignature::new(public_nonce, u, v))
}

#[cfg(test)]
mod tests {
    use super::*;

    mod validate_confidential_proof {
        use rand::rngs::OsRng;

        use super::*;
        use crate::crypto::generate_confidential_proof;

        fn create_valid_proof(value: u64, minimum_value_promise: u64) -> ConfidentialProof {
            let mask = PrivateKey::random(&mut OsRng);
            generate_confidential_proof(&mask, value, minimum_value_promise)
        }

        #[test]
        fn it_is_valid_if_proof_is_valid() {
            let proof = create_valid_proof(100, 0);
            validate_confidential_proof(proof).unwrap();
        }

        #[test]
        fn it_is_invalid_if_minimum_value_changed() {
            let mut proof = create_valid_proof(100, 100);
            proof.minimum_value_promise = 99;
            validate_confidential_proof(proof.clone()).unwrap_err();
            proof.minimum_value_promise = 1000;
            validate_confidential_proof(proof).unwrap_err();
        }
    }
}
