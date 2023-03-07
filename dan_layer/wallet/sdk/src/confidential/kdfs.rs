//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use chacha20poly1305::{aead::generic_array::GenericArray, Key};
use tari_common_types::types::{Commitment, PrivateKey, PublicKey};
use tari_crypto::dhke::DiffieHellmanSharedSecret;
use tari_utilities::{hidden_type, safe_array::SafeArray, ByteArray, Hidden};
use zeroize::Zeroize;

use crate::hashing::{encrypted_value_hasher, output_mask_hasher};

pub(crate) const AEAD_KEY_LEN: usize = std::mem::size_of::<Key>();
hidden_type!(EncryptedValueKey, SafeArray<u8, AEAD_KEY_LEN>);

/// Generate a ChaCha20-Poly1305 key from a private key and commitment using Blake2b
pub fn encrypted_value_kdf_aead(private_key: &PrivateKey, commitment: &Commitment) -> EncryptedValueKey {
    let mut aead_key = EncryptedValueKey::from(SafeArray::default());
    encrypted_value_hasher()
        .chain(private_key)
        .chain(commitment)
        .finalize_into(GenericArray::from_mut_slice(aead_key.reveal_mut()));
    aead_key
}

hidden_type!(OutputMask, SafeArray<u8, 32>);
/// Generate an output mask from a shared secret
pub fn output_mask_kdf(shared_secret: &DiffieHellmanSharedSecret<PublicKey>) -> PrivateKey {
    let mut key = OutputMask::from(SafeArray::default());
    output_mask_hasher()
        .chain(shared_secret.as_bytes())
        .finalize_into(GenericArray::from_mut_slice(key.reveal_mut()));
    PrivateKey::from_bytes(key.reveal()).unwrap()
}
