//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use rand::rngs::OsRng;
use tari_common_types::types::{PrivateKey, PublicKey};
use tari_crypto::keys::PublicKey as PublicKeyT;

pub fn create_key_pair() -> (PrivateKey, PublicKey) {
    PublicKey::random_keypair(&mut OsRng)
}
