//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::Deserialize;
use tari_ootle_transaction::{
    TransactionSealSignature,
    TransactionSignature,
    UnsealedTransactionV1,
    UnsignedTransactionV1,
};
use tari_template_lib_types::crypto::SchnorrSignatureBytes;

use crate::{error::OotleWasmError, hash::public_key_bytes_from_bytes, keys::secret_key_from_bytes};

/// An unsigned or unsealed transaction parsed from JSON.
#[derive(Deserialize)]
#[serde(untagged)]
enum TransactionInput {
    Unsealed(UnsealedTransactionV1),
    Unsigned(UnsignedTransactionV1),
}

impl TransactionInput {
    fn into_unsealed(self) -> UnsealedTransactionV1 {
        match self {
            Self::Unsigned(tx) => UnsealedTransactionV1::new(tx, vec![]),
            Self::Unsealed(tx) => tx,
        }
    }
}

fn parse_transaction_json(tx_json: &str) -> Result<TransactionInput, OotleWasmError> {
    serde_json::from_str(tx_json).map_err(Into::into)
}

/// Parse unsigned or unsealed transaction JSON as an unsealed transaction.
pub(crate) fn parse_unsealed_transaction_json(tx_json: &str) -> Result<UnsealedTransactionV1, OotleWasmError> {
    Ok(parse_transaction_json(tx_json)?.into_unsealed())
}

/// Converts a raw 64-byte signature (`R ‖ s`) into the signature type a transaction carries.
fn signature_from_bytes(bytes: &[u8]) -> Result<SchnorrSignatureBytes, OotleWasmError> {
    SchnorrSignatureBytes::from_bytes(bytes).map_err(|_| OotleWasmError::InvalidByteLength {
        field: "signature",
        expected: SchnorrSignatureBytes::length(),
        got: bytes.len(),
    })
}

/// Seal a transaction (accepts unsigned or unsealed JSON) with the seal signer's secret key.
///
/// Returns the sealed `Transaction` as a JSON string.
pub fn seal_transaction_json(tx_json: &str, seal_signer_secret_key: &[u8]) -> Result<String, OotleWasmError> {
    let unsealed = parse_transaction_json(tx_json)?.into_unsealed();
    let secret = secret_key_from_bytes(seal_signer_secret_key)?;
    let sealed = unsealed.seal(&secret);
    Ok(serde_json::to_string(&sealed)?)
}

/// Add a signer to a transaction (accepts unsigned or unsealed JSON).
///
/// Returns the unsealed transaction (with the new signature appended) as a JSON string.
pub fn add_transaction_signer_json(
    tx_json: &str,
    signer_secret_key: &[u8],
    seal_signer_public_key: &[u8],
) -> Result<String, OotleWasmError> {
    let unsealed = parse_transaction_json(tx_json)?.into_unsealed();
    let secret = secret_key_from_bytes(signer_secret_key)?;
    let seal_signer = public_key_bytes_from_bytes(seal_signer_public_key)?;
    let unsealed = unsealed.add_signer(&seal_signer, &secret);
    Ok(serde_json::to_string(&unsealed)?)
}

/// Seal a transaction (accepts unsigned or unsealed JSON) with a signature produced outside this crate,
/// e.g. by an HSM or MPC signer, over the message from `hash_unsealed_transaction_json`.
///
/// `signature` is the raw 64-byte `R ‖ s` encoding. The seal and every authorization signature already on
/// the transaction are verified, so a mismatch fails at attach time.
///
/// Returns the sealed `Transaction` as a JSON string.
pub fn attach_seal_signature_json(
    tx_json: &str,
    seal_signer_public_key: &[u8],
    signature: &[u8],
) -> Result<String, OotleWasmError> {
    let unsealed = parse_unsealed_transaction_json(tx_json)?;
    let seal_signer = public_key_bytes_from_bytes(seal_signer_public_key)?;
    let seal_signature = TransactionSealSignature::new(seal_signer, signature_from_bytes(signature)?);
    if !seal_signature.verify_v1(&unsealed) {
        return Err(OotleWasmError::SealSignatureInvalid);
    }
    if !unsealed.verify_all_signatures(&seal_signer) {
        return Err(OotleWasmError::AuthorizationForOtherSealSigner);
    }
    Ok(serde_json::to_string(&unsealed.seal_with_signature(seal_signature))?)
}

/// Append an authorization signature produced outside this crate, e.g. by an HSM or MPC signer, over the
/// message from `hash_unsigned_transaction_json` for `seal_signer_public_key`.
///
/// `signature` is the raw 64-byte `R ‖ s` encoding, verified before it is appended.
///
/// Returns the unsealed transaction (with the new signature appended) as a JSON string.
pub fn attach_transaction_signature_json(
    tx_json: &str,
    signer_public_key: &[u8],
    signature: &[u8],
    seal_signer_public_key: &[u8],
) -> Result<String, OotleWasmError> {
    let unsealed = parse_unsealed_transaction_json(tx_json)?;
    let signer = public_key_bytes_from_bytes(signer_public_key)?;
    let seal_signer = public_key_bytes_from_bytes(seal_signer_public_key)?;
    let transaction_signature = TransactionSignature::new(signer, signature_from_bytes(signature)?);
    if !transaction_signature.verify_v1(&seal_signer, unsealed.unsigned_transaction()) {
        return Err(OotleWasmError::TransactionSignatureInvalid);
    }
    let (unsigned, mut signatures) = unsealed.into_parts();
    signatures.push(transaction_signature);
    Ok(serde_json::to_string(&UnsealedTransactionV1::new(
        unsigned, signatures,
    ))?)
}

#[cfg(test)]
mod tests {
    use ootle_byte_type::ToByteType;
    use tari_crypto::{
        keys::{PublicKey, SecretKey},
        ristretto::{RistrettoPublicKey, RistrettoSchnorr, RistrettoSecretKey},
        tari_utilities::ByteArray,
    };
    use tari_engine_types::Epoch;
    use tari_ootle_transaction::{Transaction, UnsignedTransactionV1};

    use super::*;
    use crate::hash::{hash_unsealed_transaction_json, hash_unsigned_transaction_json};

    fn make_unsigned_tx() -> UnsignedTransactionV1 {
        UnsignedTransactionV1::new(0u8, vec![], vec![], Default::default(), None, Epoch(1), false)
    }

    fn random_keypair() -> (RistrettoSecretKey, RistrettoPublicKey) {
        let secret = RistrettoSecretKey::random(&mut rand::rng());
        let public_key = RistrettoPublicKey::from_secret_key(&secret);
        (secret, public_key)
    }

    /// Signs `message` the way an external signer does, returning the raw 64-byte `R ‖ s` encoding.
    fn external_sign(secret: &RistrettoSecretKey, message: &[u8]) -> Vec<u8> {
        let signature = RistrettoSchnorr::sign(secret, message, &mut rand::rng()).unwrap();
        [
            signature.get_public_nonce().as_bytes(),
            signature.get_signature().as_bytes(),
        ]
        .concat()
    }

    #[test]
    fn seal_unsigned_transaction() {
        let secret = RistrettoSecretKey::random(&mut rand::rng());
        let unsigned_json = serde_json::to_string(&make_unsigned_tx()).unwrap();

        let sealed_json = seal_transaction_json(&unsigned_json, secret.as_bytes()).unwrap();
        let sealed: Transaction = serde_json::from_str(&sealed_json).unwrap();
        assert!(sealed.verify_all_signatures());
    }

    #[test]
    fn seal_unsealed_transaction() {
        let seal_secret = RistrettoSecretKey::random(&mut rand::rng());
        let signer_secret = RistrettoSecretKey::random(&mut rand::rng());
        let seal_pk = RistrettoPublicKey::from_secret_key(&seal_secret);

        // First add a signer to get an unsealed tx
        let unsigned_json = serde_json::to_string(&make_unsigned_tx()).unwrap();
        let unsealed_json =
            add_transaction_signer_json(&unsigned_json, signer_secret.as_bytes(), seal_pk.as_bytes()).unwrap();

        // Now seal the unsealed tx
        let sealed_json = seal_transaction_json(&unsealed_json, seal_secret.as_bytes()).unwrap();
        let sealed: Transaction = serde_json::from_str(&sealed_json).unwrap();
        assert!(sealed.verify_all_signatures());
    }

    #[test]
    fn add_signer_to_unsigned_transaction() {
        let seal_secret = RistrettoSecretKey::random(&mut rand::rng());
        let seal_pk = RistrettoPublicKey::from_secret_key(&seal_secret);
        let signer_secret = RistrettoSecretKey::random(&mut rand::rng());

        let unsigned_json = serde_json::to_string(&make_unsigned_tx()).unwrap();
        let unsealed_json =
            add_transaction_signer_json(&unsigned_json, signer_secret.as_bytes(), seal_pk.as_bytes()).unwrap();

        let unsealed: UnsealedTransactionV1 = serde_json::from_str(&unsealed_json).unwrap();
        assert_eq!(unsealed.signatures().len(), 1);
        assert!(unsealed.verify_all_signatures(&seal_pk.to_byte_type()));
    }

    #[test]
    fn add_multiple_signers() {
        let seal_secret = RistrettoSecretKey::random(&mut rand::rng());
        let seal_pk = RistrettoPublicKey::from_secret_key(&seal_secret);
        let signer1 = RistrettoSecretKey::random(&mut rand::rng());
        let signer2 = RistrettoSecretKey::random(&mut rand::rng());

        let unsigned_json = serde_json::to_string(&make_unsigned_tx()).unwrap();

        // Add first signer (unsigned → unsealed)
        let unsealed_json =
            add_transaction_signer_json(&unsigned_json, signer1.as_bytes(), seal_pk.as_bytes()).unwrap();

        // Add second signer (unsealed → unsealed)
        let unsealed_json =
            add_transaction_signer_json(&unsealed_json, signer2.as_bytes(), seal_pk.as_bytes()).unwrap();

        // Seal and verify
        let sealed_json = seal_transaction_json(&unsealed_json, seal_secret.as_bytes()).unwrap();
        let sealed: Transaction = serde_json::from_str(&sealed_json).unwrap();
        assert!(sealed.verify_all_signatures());
        assert_eq!(sealed.signatures().len(), 2);
    }

    #[test]
    fn attach_external_seal_signature_to_unsigned_transaction() {
        let (seal_secret, seal_pk) = random_keypair();
        let unsigned_json = serde_json::to_string(&make_unsigned_tx()).unwrap();

        let hash = hash_unsealed_transaction_json(&unsigned_json).unwrap();
        let signature = external_sign(&seal_secret, &hash);
        let sealed_json = attach_seal_signature_json(&unsigned_json, seal_pk.as_bytes(), &signature).unwrap();

        let sealed: Transaction = serde_json::from_str(&sealed_json).unwrap();
        assert!(sealed.verify_all_signatures());
    }

    #[test]
    fn external_authorizer_and_external_seal_signer() {
        let (seal_secret, seal_pk) = random_keypair();
        let (signer_secret, signer_pk) = random_keypair();
        let unsigned_json = serde_json::to_string(&make_unsigned_tx()).unwrap();

        let auth_hash = hash_unsigned_transaction_json(&unsigned_json, seal_pk.as_bytes()).unwrap();
        let auth_signature = external_sign(&signer_secret, &auth_hash);
        let unsealed_json = attach_transaction_signature_json(
            &unsigned_json,
            signer_pk.as_bytes(),
            &auth_signature,
            seal_pk.as_bytes(),
        )
        .unwrap();

        let seal_hash = hash_unsealed_transaction_json(&unsealed_json).unwrap();
        let seal_signature = external_sign(&seal_secret, &seal_hash);
        let sealed_json = attach_seal_signature_json(&unsealed_json, seal_pk.as_bytes(), &seal_signature).unwrap();

        let sealed: Transaction = serde_json::from_str(&sealed_json).unwrap();
        assert!(sealed.verify_all_signatures());
        assert_eq!(sealed.signatures().len(), 1);
    }

    #[test]
    fn attach_seal_signature_rejects_wrong_length() {
        let (seal_secret, seal_pk) = random_keypair();
        let unsigned_json = serde_json::to_string(&make_unsigned_tx()).unwrap();
        let hash = hash_unsealed_transaction_json(&unsigned_json).unwrap();
        let signature = external_sign(&seal_secret, &hash);

        let err = attach_seal_signature_json(&unsigned_json, seal_pk.as_bytes(), &signature[..63]).unwrap_err();
        assert!(matches!(err, OotleWasmError::InvalidByteLength {
            field: "signature",
            expected: 64,
            got: 63
        }));
    }

    #[test]
    fn attach_seal_signature_rejects_wrong_key() {
        let (seal_secret, _) = random_keypair();
        let (_, other_pk) = random_keypair();
        let unsigned_json = serde_json::to_string(&make_unsigned_tx()).unwrap();
        let hash = hash_unsealed_transaction_json(&unsigned_json).unwrap();
        let signature = external_sign(&seal_secret, &hash);

        let err = attach_seal_signature_json(&unsigned_json, other_pk.as_bytes(), &signature).unwrap_err();
        assert!(matches!(err, OotleWasmError::SealSignatureInvalid));
    }

    #[test]
    fn attach_seal_signature_rejects_signature_over_authorization_message() {
        let (seal_secret, seal_pk) = random_keypair();
        let unsigned_json = serde_json::to_string(&make_unsigned_tx()).unwrap();
        let auth_hash = hash_unsigned_transaction_json(&unsigned_json, seal_pk.as_bytes()).unwrap();
        let signature = external_sign(&seal_secret, &auth_hash);

        let err = attach_seal_signature_json(&unsigned_json, seal_pk.as_bytes(), &signature).unwrap_err();
        assert!(matches!(err, OotleWasmError::SealSignatureInvalid));
    }

    #[test]
    fn attach_seal_signature_rejects_hash_taken_before_authorization() {
        let (seal_secret, seal_pk) = random_keypair();
        let (signer_secret, _) = random_keypair();
        let unsigned_json = serde_json::to_string(&make_unsigned_tx()).unwrap();
        let early_hash = hash_unsealed_transaction_json(&unsigned_json).unwrap();
        let unsealed_json =
            add_transaction_signer_json(&unsigned_json, signer_secret.as_bytes(), seal_pk.as_bytes()).unwrap();

        let signature = external_sign(&seal_secret, &early_hash);
        let err = attach_seal_signature_json(&unsealed_json, seal_pk.as_bytes(), &signature).unwrap_err();
        assert!(matches!(err, OotleWasmError::SealSignatureInvalid));
    }

    #[test]
    fn attach_seal_signature_rejects_authorization_for_other_seal_signer() {
        let (seal_secret, seal_pk) = random_keypair();
        let (_, other_seal_pk) = random_keypair();
        let (signer_secret, _) = random_keypair();
        let unsigned_json = serde_json::to_string(&make_unsigned_tx()).unwrap();
        let unsealed_json =
            add_transaction_signer_json(&unsigned_json, signer_secret.as_bytes(), other_seal_pk.as_bytes()).unwrap();

        let hash = hash_unsealed_transaction_json(&unsealed_json).unwrap();
        let signature = external_sign(&seal_secret, &hash);
        let err = attach_seal_signature_json(&unsealed_json, seal_pk.as_bytes(), &signature).unwrap_err();
        assert!(matches!(err, OotleWasmError::AuthorizationForOtherSealSigner));
    }

    #[test]
    fn attach_transaction_signature_rejects_wrong_key() {
        let (_, seal_pk) = random_keypair();
        let (signer_secret, _) = random_keypair();
        let (_, other_pk) = random_keypair();
        let unsigned_json = serde_json::to_string(&make_unsigned_tx()).unwrap();
        let hash = hash_unsigned_transaction_json(&unsigned_json, seal_pk.as_bytes()).unwrap();
        let signature = external_sign(&signer_secret, &hash);

        let err =
            attach_transaction_signature_json(&unsigned_json, other_pk.as_bytes(), &signature, seal_pk.as_bytes())
                .unwrap_err();
        assert!(matches!(err, OotleWasmError::TransactionSignatureInvalid));
    }

    #[test]
    fn attach_transaction_signature_rejects_signature_over_seal_message() {
        let (_, seal_pk) = random_keypair();
        let (signer_secret, signer_pk) = random_keypair();
        let unsigned_json = serde_json::to_string(&make_unsigned_tx()).unwrap();
        let seal_hash = hash_unsealed_transaction_json(&unsigned_json).unwrap();
        let signature = external_sign(&signer_secret, &seal_hash);

        let err =
            attach_transaction_signature_json(&unsigned_json, signer_pk.as_bytes(), &signature, seal_pk.as_bytes())
                .unwrap_err();
        assert!(matches!(err, OotleWasmError::TransactionSignatureInvalid));
    }

    #[test]
    fn attach_transaction_signature_rejects_wrong_length() {
        let (_, seal_pk) = random_keypair();
        let (_, signer_pk) = random_keypair();
        let unsigned_json = serde_json::to_string(&make_unsigned_tx()).unwrap();

        let err =
            attach_transaction_signature_json(&unsigned_json, signer_pk.as_bytes(), &[0u8; 65], seal_pk.as_bytes())
                .unwrap_err();
        assert!(matches!(err, OotleWasmError::InvalidByteLength {
            field: "signature",
            expected: 64,
            got: 65
        }));
    }
}
