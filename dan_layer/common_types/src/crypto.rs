//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use rand::rngs::OsRng;
use tari_common_types::types::{PrivateKey, PublicKey};
use tari_crypto::{
    keys::{PublicKey as PublicKeyT, SecretKey},
    tari_utilities::hex::Hex,
};

pub fn create_key_pair() -> (PrivateKey, PublicKey) {
    PublicKey::random_keypair(&mut OsRng)
}

pub fn create_secret() -> String {
    let (secret, _) = create_key_pair();
    secret.to_hex()
}

pub fn create_key_pair_from_seed(seed: u8) -> (PrivateKey, PublicKey) {
    let private = PrivateKey::from_uniform_bytes(&[seed; 64]).unwrap();
    let public = PublicKey::from_secret_key(&private);
    (private, public)
}
