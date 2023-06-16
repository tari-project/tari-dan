//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use rand::rngs::OsRng;
use tari_common_types::types::{FixedHash, PrivateKey, PublicKey, Signature};
use tari_consensus::traits::{SigningService, VoteSigningService};
use tari_crypto::keys::PublicKey as _;
use tari_dan_common_types::ShardId;
use tari_dan_storage::consensus_models::{BlockId, QuorumDecision};

#[derive(Debug, Clone)]
pub struct TestVoteSigningService {
    pub public_key: PublicKey,
    pub secret_key: PrivateKey,
}

impl TestVoteSigningService {
    pub fn new() -> Self {
        let (secret_key, public_key) = PublicKey::random_keypair(&mut OsRng);
        Self { public_key, secret_key }
    }
}

impl SigningService for TestVoteSigningService {
    fn sign(&self, _challenge: &[u8]) -> Signature {
        Signature::default()
    }

    fn verify(&self, _signature: &Signature, _challenge: &[u8]) -> bool {
        true
    }

    fn verify_for_public_key(&self, _public_key: &PublicKey, _signature: &Signature, _challenge: &[u8]) -> bool {
        true
    }

    fn public_key(&self) -> &PublicKey {
        &self.public_key
    }
}

impl VoteSigningService for TestVoteSigningService {
    fn create_challenge(_block_id: &BlockId, _decision: QuorumDecision) -> FixedHash {
        FixedHash::zero()
    }

    fn shard_id(&self) -> ShardId {
        ShardId::zero()
    }
}
