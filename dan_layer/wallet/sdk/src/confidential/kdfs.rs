//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use chacha20poly1305::{aead::generic_array::GenericArray, Key};
use tari_common_types::types::{Commitment, PrivateKey};
use tari_engine_types::base_layer_hashing::{encrypted_data_hasher, output_mask_hasher};
use tari_utilities::{hidden_type, safe_array::SafeArray, ByteArray, Hidden};
use zeroize::Zeroize;

pub(crate) const AEAD_KEY_LEN: usize = std::mem::size_of::<Key>();
hidden_type!(EncryptedValueKey, SafeArray<u8, AEAD_KEY_LEN>);

/// Generate a ChaCha20-Poly1305 key from a private key using Blake2b
pub fn encrypted_data_kdf_aead(private_key: &PrivateKey) -> PrivateKey {
    let mut aead_key = EncryptedValueKey::from(SafeArray::default());
    encrypted_data_hasher()
        .chain(private_key)
        .finalize_into(GenericArray::from_mut_slice(aead_key.reveal_mut()));

    PrivateKey::from_bytes(aead_key.reveal()).unwrap()
}

hidden_type!(OutputMask, SafeArray<u8, 32>);
/// Generate an output mask from a shared secret
pub fn output_mask_kdf(shared_secret: &PrivateKey) -> PrivateKey {
    let mut key = OutputMask::from(SafeArray::default());
    output_mask_hasher()
        .chain(shared_secret)
        .finalize_into(GenericArray::from_mut_slice(key.reveal_mut()));
    PrivateKey::from_bytes(key.reveal()).unwrap()
}
