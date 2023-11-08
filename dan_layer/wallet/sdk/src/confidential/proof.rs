//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::mem::size_of;

use blake2::Blake2b;
use chacha20poly1305::{
    aead,
    aead::{generic_array::GenericArray, OsRng},
    consts::U32,
    AeadCore,
    AeadInPlace,
    KeyInit,
    Tag,
    XChaCha20Poly1305,
    XNonce,
};
use digest::FixedOutput;
use lazy_static::lazy_static;
use tari_common_types::types::{BulletRangeProof, Commitment, CommitmentFactory, PrivateKey, PublicKey};
use tari_crypto::{
    commitment::{ExtensionDegree, HomomorphicCommitmentFactory},
    errors::RangeProofError,
    extended_range_proof::ExtendedRangeProofService,
    hash_domain,
    hashing::DomainSeparatedHasher,
    keys::SecretKey,
    ristretto::bulletproofs_plus::{BulletproofsPlusService, RistrettoExtendedMask, RistrettoExtendedWitness},
    tari_utilities::ByteArray,
};
use tari_template_lib::{
    crypto::RistrettoPublicKeyBytes,
    models::{Amount, ConfidentialOutputProof, ConfidentialStatement, EncryptedData},
};
use tari_utilities::safe_array::SafeArray;
use zeroize::Zeroizing;

use crate::{
    byte_utils::copy_fixed,
    confidential::{error::ConfidentialProofError, kdfs::EncryptedDataKey32},
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
    pub sender_public_nonce: PublicKey,
    pub minimum_value_promise: u64,
    pub encrypted_data: EncryptedData,
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
            Ok(ConfidentialStatement {
                commitment: copy_fixed(change_commitment.as_bytes()),
                sender_public_nonce: RistrettoPublicKeyBytes::from_bytes(stmt.sender_public_nonce.as_bytes())
                    .expect("[generate_confidential_proof] change nonce"),
                encrypted_data: stmt.encrypted_data.clone(),
                minimum_value_promise: stmt.minimum_value_promise,
                revealed_amount: stmt.reveal_amount,
            })
        })
        .transpose()?;

    let commitment = output_statement.to_commitment();

    let output_range_proof = generate_extended_bullet_proof(output_statement, change_statement)?;

    Ok(ConfidentialOutputProof {
        output_statement: ConfidentialStatement {
            commitment: copy_fixed(commitment.as_bytes()),
            sender_public_nonce: RistrettoPublicKeyBytes::from_bytes(output_statement.sender_public_nonce.as_bytes())
                .expect("[generate_confidential_proof] output nonce"),
            encrypted_data: output_statement.encrypted_data.clone(),
            minimum_value_promise: output_statement.minimum_value_promise,
            revealed_amount: output_statement.reveal_amount,
        },
        change_statement: proof_change_statement,
        range_proof: output_range_proof.0,
    })
}

fn inner_encrypted_data_kdf_aead(encryption_key: &PrivateKey, commitment: &Commitment) -> EncryptedDataKey32 {
    let mut aead_key = EncryptedDataKey32::from(SafeArray::default());
    // This has to be the same as the base layer so that burn claims are spendable
    hash_domain!(
        TransactionSecureNonceKdfDomain,
        "com.tari.base_layer.core.transactions.secure_nonce_kdf",
        0
    );
    DomainSeparatedHasher::<Blake2b<U32>, TransactionSecureNonceKdfDomain>::new_with_label("encrypted_value_and_mask")
        .chain(encryption_key.as_bytes())
        .chain(commitment.as_bytes())
        .finalize_into(GenericArray::from_mut_slice(aead_key.reveal_mut()));
    aead_key
}

const ENCRYPTED_DATA_TAG: &[u8] = b"TARI_AAD_VALUE_AND_MASK_EXTEND_NONCE_VARIANT";
// Useful size constants, each in bytes
const SIZE_NONCE: usize = size_of::<XNonce>();
const SIZE_VALUE: usize = size_of::<u64>();
const SIZE_MASK: usize = PrivateKey::KEY_LEN;
const SIZE_TAG: usize = size_of::<Tag>();
const SIZE_TOTAL: usize = SIZE_NONCE + SIZE_VALUE + SIZE_MASK + SIZE_TAG;

pub(crate) fn encrypt_data(
    encryption_key: &PrivateKey,
    commitment: &Commitment,
    value: u64,
    mask: &PrivateKey,
) -> Result<EncryptedData, aead::Error> {
    // Encode the value and mask
    let mut bytes = Zeroizing::new([0u8; SIZE_VALUE + SIZE_MASK]);
    bytes[..SIZE_VALUE].clone_from_slice(value.to_le_bytes().as_ref());
    bytes[SIZE_VALUE..].clone_from_slice(mask.as_bytes());

    // Produce a secure random nonce
    let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);

    // Set up the AEAD
    let aead_key = inner_encrypted_data_kdf_aead(encryption_key, commitment);
    let cipher = XChaCha20Poly1305::new(GenericArray::from_slice(aead_key.reveal()));

    // Encrypt in place
    let tag = cipher.encrypt_in_place_detached(&nonce, ENCRYPTED_DATA_TAG, bytes.as_mut_slice())?;

    // Put everything together: nonce, ciphertext, tag
    let mut data = [0u8; SIZE_TOTAL];
    data[..SIZE_NONCE].clone_from_slice(&nonce);
    data[SIZE_NONCE..SIZE_NONCE + SIZE_VALUE + SIZE_MASK].clone_from_slice(bytes.as_slice());
    data[SIZE_NONCE + SIZE_VALUE + SIZE_MASK..].clone_from_slice(&tag);

    Ok(EncryptedData(data))
}

pub fn decrypt_data_and_mask(
    encryption_key: &PrivateKey,
    commitment: &Commitment,
    encrypted_data: &EncryptedData,
) -> Result<(u64, PrivateKey), aead::Error> {
    // Extract the nonce, ciphertext, and tag
    let nonce = XNonce::from_slice(&encrypted_data.0.as_bytes()[..SIZE_NONCE]);
    let mut bytes = Zeroizing::new([0u8; SIZE_VALUE + SIZE_MASK]);
    bytes.clone_from_slice(&encrypted_data.as_bytes()[SIZE_NONCE..SIZE_NONCE + SIZE_VALUE + SIZE_MASK]);
    let tag = Tag::from_slice(&encrypted_data.as_bytes()[SIZE_NONCE + SIZE_VALUE + SIZE_MASK..]);

    // Set up the AEAD
    let aead_key = inner_encrypted_data_kdf_aead(encryption_key, commitment);
    let cipher = XChaCha20Poly1305::new(GenericArray::from_slice(aead_key.reveal()));

    // Decrypt in place
    cipher.decrypt_in_place_detached(nonce, ENCRYPTED_DATA_TAG, bytes.as_mut_slice(), tag)?;

    // Decode the value and mask
    let mut value_bytes = [0u8; SIZE_VALUE];
    value_bytes.clone_from_slice(&bytes[0..SIZE_VALUE]);
    Ok((
        u64::from_le_bytes(value_bytes),
        PrivateKey::from_canonical_bytes(&bytes[SIZE_VALUE..]).expect("The length of bytes is exactly SIZE_MASK"),
    ))
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
                    encrypted_data: EncryptedData([0u8; EncryptedData::size()]),
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
            let mask = PrivateKey::random(&mut OsRng);
            let encrypted = encrypt_data(&key, &commitment, amount, &mask).unwrap();

            let val = decrypt_data_and_mask(&key, &commitment, &encrypted).unwrap();
            assert_eq!(val.0, 100);
        }
    }
}
