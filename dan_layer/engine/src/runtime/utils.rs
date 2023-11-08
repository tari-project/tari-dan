//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_common_types::types::PublicKey;
use tari_crypto::tari_utilities::ByteArray;
use tari_template_lib::crypto::RistrettoPublicKeyBytes;

pub fn to_ristretto_public_key_bytes(public_key: &PublicKey) -> RistrettoPublicKeyBytes {
    RistrettoPublicKeyBytes::from_bytes(public_key.as_bytes()).expect(
        "PublicKey alias is not a valid RistrettoPublicKeyBytes. This can only happen if the byte length of PublicKey \
         is not size_of::<RistrettoPublicKeyBytes>() bytes.",
    )
}
