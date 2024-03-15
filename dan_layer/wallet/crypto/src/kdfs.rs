//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use chacha20poly1305::aead::generic_array::GenericArray;
use digest::FixedOutput;
use tari_crypto::{
    dhke::DiffieHellmanSharedSecret,
    keys::SecretKey,
    ristretto::{RistrettoPublicKey, RistrettoSecretKey},
};
use tari_engine_types::base_layer_hashing::encrypted_data_hasher;
use tari_utilities::{hidden_type, safe_array::SafeArray, Hidden};
use zeroize::Zeroize;

hidden_type!(EncryptedDataKey32, SafeArray<u8, 32>);
hidden_type!(EncryptedDataKey64, SafeArray<u8, 64>);

/// Generate a ChaCha20-Poly1305 key from a private key and commitment using Blake2b
pub fn encrypted_data_dh_kdf_aead(
    private_key: &RistrettoSecretKey,
    public_nonce: &RistrettoPublicKey,
) -> RistrettoSecretKey {
    let shared_secret = DiffieHellmanSharedSecret::<RistrettoPublicKey>::new(private_key, public_nonce);
    let mut aead_key = EncryptedDataKey64::from(SafeArray::default());
    // Must match base layer burn
    encrypted_data_hasher()
        .chain(shared_secret.as_bytes())
        .finalize_into(GenericArray::from_mut_slice(aead_key.reveal_mut()));

    RistrettoSecretKey::from_uniform_bytes(aead_key.reveal()).unwrap()
}
