//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_crypto::{
    keys::{PublicKey, SecretKey},
    ristretto::{RistrettoPublicKey, RistrettoSecretKey},
    tari_utilities::ByteArray,
};
use tari_ootle_common_types::signature::TRANSACTION_SIGNING_CONTEXT;

use crate::error::OotleWasmError;

/// A generated keypair as raw bytes.
#[derive(Debug, Clone)]
pub struct KeypairResult {
    pub secret_key: Vec<u8>,
    pub public_key: Vec<u8>,
}

/// Result of a Schnorr signature operation as raw bytes.
#[derive(Debug, Clone)]
pub struct SchnorrSignatureResult {
    /// The public nonce commitment.
    pub public_nonce: Vec<u8>,
    /// The signature scalar.
    pub signature: Vec<u8>,
}

/// Generate a new random Ristretto keypair.
pub fn generate_keypair() -> KeypairResult {
    let secret_key = RistrettoSecretKey::random(&mut rand::rng());
    let public_key = RistrettoPublicKey::from_secret_key(&secret_key);
    KeypairResult {
        secret_key: secret_key.as_bytes().to_vec(),
        public_key: public_key.as_bytes().to_vec(),
    }
}

/// Sign a transaction signing message (the 64-byte transaction hash) with a Ristretto secret key,
/// both as raw bytes. Returns the 64-byte sr25519 signature.
pub fn sr25519_sign(secret_key: &[u8], message: &[u8]) -> Result<Vec<u8>, OotleWasmError> {
    let secret_key = RistrettoSecretKey::from_canonical_bytes(secret_key)
        .map_err(|e| OotleWasmError::InvalidSecretKey(e.to_string()))?;
    Ok(TRANSACTION_SIGNING_CONTEXT
        .sign(&secret_key, message)
        .as_bytes()
        .to_vec())
}

/// Derive the public key from a secret key (both as raw bytes).
pub fn public_key_from_secret_key(secret_key: &[u8]) -> Result<Vec<u8>, OotleWasmError> {
    let secret_key = RistrettoSecretKey::from_canonical_bytes(secret_key)
        .map_err(|e| OotleWasmError::InvalidSecretKey(e.to_string()))?;
    let public_key = RistrettoPublicKey::from_secret_key(&secret_key);
    Ok(public_key.as_bytes().to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_and_verify_round_trip() {
        use ootle_byte_type::ToByteType;
        use tari_ootle_common_types::signature::Sr25519SignatureBytes;

        let secret = RistrettoSecretKey::random(&mut rand::rng());
        let public_key = RistrettoPublicKey::from_secret_key(&secret);
        let message = [1u8; 64];

        let signature: [u8; 64] = sr25519_sign(secret.as_bytes(), &message).unwrap().try_into().unwrap();
        assert!(TRANSACTION_SIGNING_CONTEXT.verify(
            &Sr25519SignatureBytes::from_array(signature),
            &public_key.to_byte_type(),
            &message
        ));
    }

    #[test]
    fn public_key_derivation() {
        let secret = RistrettoSecretKey::random(&mut rand::rng());
        let expected = RistrettoPublicKey::from_secret_key(&secret);

        let result = public_key_from_secret_key(secret.as_bytes()).unwrap();
        assert_eq!(result, expected.as_bytes());
    }

    #[test]
    fn generate_keypair_is_valid() {
        let kp = generate_keypair();

        // Keys should be 32 bytes
        assert_eq!(kp.secret_key.len(), 32);
        assert_eq!(kp.public_key.len(), 32);

        // Deriving public key from the generated secret key should match
        let derived = public_key_from_secret_key(&kp.secret_key).unwrap();
        assert_eq!(derived, kp.public_key);
    }

    #[test]
    fn generate_keypair_is_unique() {
        let kp1 = generate_keypair();
        let kp2 = generate_keypair();
        assert_ne!(kp1.secret_key, kp2.secret_key);
        assert_ne!(kp1.public_key, kp2.public_key);
    }
}
