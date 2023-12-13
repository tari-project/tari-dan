//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_crypto::{ristretto::RistrettoPublicKey, tari_utilities::ByteArray};
use tari_template_lib::crypto::RistrettoPublicKeyBytes;

pub fn public_key_to_ristretto_bytes(public_key: &RistrettoPublicKey) -> RistrettoPublicKeyBytes {
    RistrettoPublicKeyBytes::from_bytes(public_key.as_bytes()).unwrap()
}
