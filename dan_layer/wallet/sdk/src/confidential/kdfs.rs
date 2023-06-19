//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use chacha20poly1305::{aead::generic_array::GenericArray, Key};
use tari_common_types::types::{PrivateKey, PublicKey};
use tari_crypto::dhke::DiffieHellmanSharedSecret;
use tari_engine_types::base_layer_hashing::encrypted_data_hasher;
use tari_utilities::{hidden_type, safe_array::SafeArray, ByteArray, Hidden};
use zeroize::Zeroize;

pub(crate) const AEAD_KEY_LEN: usize = std::mem::size_of::<Key>();
hidden_type!(EncryptedDataKey, SafeArray<u8, AEAD_KEY_LEN>);

/// Generate a ChaCha20-Poly1305 key from a private key and commitment using Blake2b
pub fn encrypted_data_dh_kdf_aead(private_key: &PrivateKey, public_nonce: &PublicKey) -> PrivateKey {
    let shared_secret = DiffieHellmanSharedSecret::<PublicKey>::new(private_key, public_nonce);
    let mut aead_key = EncryptedDataKey::from(SafeArray::default());
    // Must match base layer burn
    encrypted_data_hasher()
        .chain(&shared_secret.as_bytes())
        .finalize_into(GenericArray::from_mut_slice(aead_key.reveal_mut()));

    PrivateKey::from_bytes(aead_key.reveal()).unwrap()
}
