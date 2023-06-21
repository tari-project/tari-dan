//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use rand::rngs::OsRng;
use tari_common_types::types::{PrivateKey, PublicKey};
use tari_consensus::traits::{ValidatorSignatureService, VoteSignatureService};
use tari_crypto::keys::PublicKey as _;
use tari_dan_storage::consensus_models::ValidatorSchnorrSignature;

#[derive(Debug, Clone)]
pub struct TestVoteSignatureService {
    pub public_key: PublicKey,
    pub secret_key: PrivateKey,
}

impl TestVoteSignatureService {
    pub fn new() -> Self {
        let (secret_key, public_key) = PublicKey::random_keypair(&mut OsRng);
        Self { public_key, secret_key }
    }
}

impl ValidatorSignatureService for TestVoteSignatureService {
    fn sign<M: AsRef<[u8]>>(&self, message: M) -> ValidatorSchnorrSignature {
        ValidatorSchnorrSignature::sign_message(&self.secret_key, message).unwrap()
    }

    fn public_key(&self) -> &PublicKey {
        &self.public_key
    }
}

impl VoteSignatureService for TestVoteSignatureService {}
