//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Key-derivation primitives for stealth transfers.

use ootle_network::Network;
use tari_crypto::tari_utilities::ByteArray;
use tari_ootle_wallet_crypto::kdfs::{encrypted_data_dh_kdf_aead, owner_stealth_dh_secret};

use crate::{
    error::OotleWasmError,
    keys::{public_key_from_bytes, secret_key_from_bytes},
};

/// Derive the recipient's stealth spending scalar `c + k`, where `c = H(network || v·R)`, `k` is the
/// account secret, `v` the view secret and `R` the sender-provided public nonce.
///
/// The receiver runs this over each candidate UTXO to produce the one-time secret that controls the
/// stealth output.
pub fn stealth_dh_secret(
    network_byte: u8,
    account_secret: &[u8],
    view_secret: &[u8],
    public_nonce: &[u8],
) -> Result<Vec<u8>, OotleWasmError> {
    let network = Network::try_from(network_byte).map_err(|e| OotleWasmError::InvalidNetwork(e.to_string()))?;
    let account = secret_key_from_bytes(account_secret)?;
    let view = secret_key_from_bytes(view_secret)?;
    let nonce = public_key_from_bytes(public_nonce)?;
    let stealth = owner_stealth_dh_secret(network, &account, &view, &nonce);
    Ok(stealth.as_bytes().to_vec())
}

/// Derive the AEAD encryption key from a Diffie-Hellman shared secret: `H(DH(s, P))`.
///
/// Sender derives it with `(sender_secret_nonce, recipient_view_pub)`, receiver derives the same key with
/// `(recipient_view_secret, sender_public_nonce)`.
pub fn encrypted_data_dh_kdf(private_key: &[u8], public_key: &[u8]) -> Result<Vec<u8>, OotleWasmError> {
    let private = secret_key_from_bytes(private_key)?;
    let public = public_key_from_bytes(public_key)?;
    Ok(encrypted_data_dh_kdf_aead(&private, &public).as_bytes().to_vec())
}

#[cfg(test)]
mod tests {
    use tari_crypto::{
        keys::{PublicKey, SecretKey},
        ristretto::{RistrettoPublicKey, RistrettoSecretKey},
    };

    use super::*;

    #[test]
    fn stealth_dh_secret_matches_the_sender_side_spend_key() {
        let network = Network::LocalNet;
        let (account_sk, account_pk) = RistrettoPublicKey::random_keypair(&mut rand::rng());
        let (view_sk, view_pk) = RistrettoPublicKey::random_keypair(&mut rand::rng());
        let (nonce_sk, nonce_pk) = RistrettoPublicKey::random_keypair(&mut rand::rng());

        let ours = stealth_dh_secret(
            network.as_byte(),
            account_sk.as_bytes(),
            view_sk.as_bytes(),
            nonce_pk.as_bytes(),
        )
        .unwrap();
        let expected = owner_stealth_dh_secret(network, &account_sk, &view_sk, &nonce_pk);
        assert_eq!(ours, expected.as_bytes().to_vec());

        let stealth_secret = secret_key_from_bytes(&ours).unwrap();
        let sender_side =
            tari_ootle_wallet_crypto::kdfs::owner_stealth_dh_stealth_address(network, &account_pk, &view_pk, &nonce_sk);
        assert_eq!(RistrettoPublicKey::from_secret_key(&stealth_secret), sender_side);
    }

    #[test]
    fn stealth_dh_secret_rejects_invalid_network() {
        let secret = RistrettoSecretKey::random(&mut rand::rng());
        let pk = RistrettoPublicKey::from_secret_key(&secret);
        let err = stealth_dh_secret(0xFF, secret.as_bytes(), secret.as_bytes(), pk.as_bytes()).unwrap_err();
        assert!(matches!(err, OotleWasmError::InvalidNetwork(_)));
    }

    #[test]
    fn encrypted_data_dh_kdf_round_trips() {
        let (alice_sk, alice_pk) = RistrettoPublicKey::random_keypair(&mut rand::rng());
        let (bob_sk, bob_pk) = RistrettoPublicKey::random_keypair(&mut rand::rng());

        let from_alice = encrypted_data_dh_kdf(alice_sk.as_bytes(), bob_pk.as_bytes()).unwrap();
        let from_bob = encrypted_data_dh_kdf(bob_sk.as_bytes(), alice_pk.as_bytes()).unwrap();
        assert_eq!(from_alice, from_bob);
        assert_eq!(from_alice.len(), 32);
    }
}
