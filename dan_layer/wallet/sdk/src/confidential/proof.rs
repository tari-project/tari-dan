//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use chacha20poly1305::{
    aead,
    aead::{generic_array::GenericArray, Aead, Payload},
    ChaCha20Poly1305,
    KeyInit,
    Nonce,
};
use digest::FixedOutput;
use lazy_static::lazy_static;
use tari_common_types::types::{BulletRangeProof, Commitment, CommitmentFactory, PrivateKey, PublicKey};
use tari_crypto::{
    commitment::{ExtensionDegree, HomomorphicCommitmentFactory},
    errors::RangeProofError,
    extended_range_proof::ExtendedRangeProofService,
    hash::blake2::Blake256,
    hash_domain,
    hashing::DomainSeparatedHasher,
    ristretto::bulletproofs_plus::{BulletproofsPlusService, RistrettoExtendedMask, RistrettoExtendedWitness},
    tari_utilities::ByteArray,
};
use tari_template_lib::{
    crypto::RistrettoPublicKeyBytes,
    models::{Amount, ConfidentialOutputProof, ConfidentialStatement, EncryptedValue},
};
use tari_utilities::safe_array::SafeArray;

use crate::{
    byte_utils::copy_fixed,
    confidential::{error::ConfidentialProofError, kdfs, kdfs::EncryptedValueKey},
};

lazy_static! {
    /// Static reference to the default commitment factory. Each instance of CommitmentFactory requires a number of heap allocations.
    static ref COMMITMENT_FACTORY: CommitmentFactory = CommitmentFactory::default();
    /// Static reference to the default range proof service. Each instance of RangeProofService requires a number of heap allocations.
    static ref RANGE_PROOF_AGG_1_SERVICE: BulletproofsPlusService =
        BulletproofsPlusService::init(64, 1, CommitmentFactory::default()).unwrap();
    static ref RANGE_PROOF_AGG_2_SERVICE: BulletproofsPlusService =
        BulletproofsPlusService::init(64, 2, CommitmentFactory::default()).unwrap();
}

pub fn get_range_proof_service(aggregation_factor: usize) -> &'static BulletproofsPlusService {
    match aggregation_factor {
        1 => &RANGE_PROOF_AGG_1_SERVICE,
        2 => &RANGE_PROOF_AGG_2_SERVICE,
        _ => panic!(
            "Unsupported BP aggregation factor {}. Expected 1 or 2",
            aggregation_factor
        ),
    }
}

pub fn get_commitment_factory() -> &'static CommitmentFactory {
    &COMMITMENT_FACTORY
}

pub struct ConfidentialProofStatement {
    pub amount: Amount,
    pub mask: PrivateKey,
    pub sender_public_nonce: Option<PublicKey>,
    pub minimum_value_promise: u64,
    pub reveal_amount: Amount,
}

impl ConfidentialProofStatement {
    pub fn to_commitment(&self) -> Commitment {
        get_commitment_factory().commit_value(&self.mask, self.amount.value() as u64)
    }
}

pub fn generate_confidential_proof(
    output_statement: &ConfidentialProofStatement,
    change_statement: Option<&ConfidentialProofStatement>,
) -> Result<ConfidentialOutputProof, ConfidentialProofError> {
    let proof_change_statement = change_statement
        .as_ref()
        .map(|stmt| -> Result<_, ConfidentialProofError> {
            let change_commitment = stmt.to_commitment();
            let encrypted_value = encrypt_value(&stmt.mask, &change_commitment, stmt.amount.value() as u64)?;
            Ok(ConfidentialStatement {
                commitment: copy_fixed(change_commitment.as_bytes()),
                sender_public_nonce: stmt.sender_public_nonce.as_ref().map(|nonce| {
                    RistrettoPublicKeyBytes::from_bytes(nonce.as_bytes())
                        .expect("[generate_confidential_proof] change nonce")
                }),
                encrypted_value,
                minimum_value_promise: stmt.minimum_value_promise,
                revealed_amount: stmt.reveal_amount,
            })
        })
        .transpose()?;

    let commitment = output_statement.to_commitment();
    let encryption_key = kdfs::encrypted_value_kdf_aead(&output_statement.mask, &commitment);
    let encrypted_value = encrypt_value(&encryption_key, &commitment, output_statement.amount.value() as u64)?;
    let output_range_proof = generate_extended_bullet_proof(output_statement, change_statement)?;

    Ok(ConfidentialOutputProof {
        output_statement: ConfidentialStatement {
            commitment: copy_fixed(commitment.as_bytes()),
            sender_public_nonce: output_statement.sender_public_nonce.as_ref().map(|nonce| {
                RistrettoPublicKeyBytes::from_bytes(nonce.as_bytes())
                    .expect("[generate_confidential_proof] output nonce")
            }),
            encrypted_value,
            minimum_value_promise: output_statement.minimum_value_promise,
            revealed_amount: output_statement.reveal_amount,
        },
        change_statement: proof_change_statement,
        range_proof: output_range_proof.0,
    })
}

fn inner_encrypted_value_kdf_aead(encryption_key: &PrivateKey, commitment: &Commitment) -> EncryptedValueKey {
    let mut aead_key = EncryptedValueKey::from(SafeArray::default());
    // This has to be the same as the base layer so that burn claims are spendable
    hash_domain!(TransactionKdfDomain, "com.tari.base_layer.core.transactions.kdf", 0);
    DomainSeparatedHasher::<Blake256, TransactionKdfDomain>::new_with_label("encrypted_value")
        .chain(encryption_key.as_bytes())
        .chain(commitment.as_bytes())
        .finalize_into(GenericArray::from_mut_slice(aead_key.reveal_mut()));
    aead_key
}

const ENCRYPTED_VALUE_TAG: &[u8] = b"TARI_AAD_VALUE";
fn encrypt_value(
    encryption_key: &PrivateKey,
    commitment: &Commitment,
    amount: u64,
) -> Result<EncryptedValue, aead::Error> {
    let aead_key = inner_encrypted_value_kdf_aead(encryption_key, commitment);
    let chacha_poly = ChaCha20Poly1305::new(GenericArray::from_slice(aead_key.reveal()));
    let payload = Payload {
        msg: &amount.to_le_bytes(),
        aad: ENCRYPTED_VALUE_TAG,
    };
    // Encrypt the value (with fixed length) using ChaCha20-Poly1305 with a fixed zero nonce
    let buffer = chacha_poly.encrypt(&Nonce::default(), payload)?;
    let mut data: [u8; EncryptedValue::size()] = [0; EncryptedValue::size()];
    data[..].copy_from_slice(&buffer);
    Ok(EncryptedValue(data))
}

pub fn decrypt_value(
    encryption_key: &PrivateKey,
    commitment: &Commitment,
    encrypted_value: &EncryptedValue,
) -> Result<u64, aead::Error> {
    let aead_key = inner_encrypted_value_kdf_aead(encryption_key, commitment);
    // Authenticate and decrypt the value
    let aead_payload = Payload {
        msg: encrypted_value.as_ref(),
        aad: ENCRYPTED_VALUE_TAG,
    };
    let mut value_bytes = [0u8; 8];
    let decrypted_bytes =
        ChaCha20Poly1305::new(GenericArray::from_slice(aead_key.reveal())).decrypt(&Nonce::default(), aead_payload)?;
    value_bytes.clone_from_slice(&decrypted_bytes[..8]);
    Ok(u64::from_le_bytes(value_bytes))
}

fn generate_extended_bullet_proof(
    output_statement: &ConfidentialProofStatement,
    change_statement: Option<&ConfidentialProofStatement>,
) -> Result<BulletRangeProof, RangeProofError> {
    let mut extended_witnesses = vec![];

    let extended_mask =
        RistrettoExtendedMask::assign(ExtensionDegree::DefaultPedersen, vec![output_statement.mask.clone()]).unwrap();

    let mut agg_factor = 1;
    extended_witnesses.push(RistrettoExtendedWitness {
        mask: extended_mask,
        value: output_statement.amount.value() as u64,
        minimum_value_promise: output_statement.minimum_value_promise,
    });
    if let Some(stmt) = change_statement {
        let extended_mask =
            RistrettoExtendedMask::assign(ExtensionDegree::DefaultPedersen, vec![stmt.mask.clone()]).unwrap();
        extended_witnesses.push(RistrettoExtendedWitness {
            mask: extended_mask,
            value: stmt.amount.value() as u64,
            minimum_value_promise: stmt.minimum_value_promise,
        });
        agg_factor = 2;
    }

    let output_range_proof = get_range_proof_service(agg_factor).construct_extended_proof(extended_witnesses, None)?;
    Ok(BulletRangeProof(output_range_proof))
}

#[cfg(test)]
mod tests {
    use rand::rngs::OsRng;
    use tari_common_types::types::PrivateKey;
    use tari_crypto::keys::SecretKey;
    use tari_engine_types::confidential::validate_confidential_proof;
    use tari_template_lib::models::Amount;

    use super::*;

    mod confidential_proof {
        use super::*;

        fn create_valid_proof(amount: Amount, minimum_value_promise: u64) -> ConfidentialOutputProof {
            let mask = PrivateKey::random(&mut OsRng);
            generate_confidential_proof(
                &ConfidentialProofStatement {
                    amount,
                    minimum_value_promise,
                    mask,
                    sender_public_nonce: Default::default(),
                    reveal_amount: Default::default(),
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

    mod encrypt_decrypt {
        use super::*;

        #[test]
        fn it_encrypts_and_decrypts() {
            let key = PrivateKey::random(&mut OsRng);
            let amount = 100;
            let commitment = get_commitment_factory().commit_value(&key, amount);
            let encrypted = encrypt_value(&key, &commitment, amount).unwrap();

            let val = decrypt_value(&key, &commitment, &encrypted).unwrap();
            assert_eq!(val, 100);
        }
    }
}
