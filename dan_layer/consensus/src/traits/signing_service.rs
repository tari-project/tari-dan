//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_common_types::types::{FixedHash, PublicKey, Signature};
use tari_dan_common_types::ShardId;
use tari_dan_storage::consensus_models::{BlockId, QuorumDecision, ValidatorSignature};

pub trait SigningService {
    fn sign(&self, challenge: &[u8]) -> Signature;
    fn verify(&self, signature: &Signature, challenge: &[u8]) -> bool;
    fn verify_for_public_key(&self, public_key: &PublicKey, signature: &Signature, challenge: &[u8]) -> bool;
    fn public_key(&self) -> &PublicKey;
}

pub trait VoteSigningService: SigningService {
    fn create_challenge(block_id: &BlockId, decision: QuorumDecision) -> FixedHash;
    fn shard_id(&self) -> ShardId;

    fn sign_vote(&self, block_id: &BlockId, decision: QuorumDecision) -> ValidatorSignature {
        let challenge = Self::create_challenge(block_id, decision);
        let signature = self.sign(&*challenge);
        ValidatorSignature {
            public_key: self.public_key().clone(),
            shard_id: self.shard_id(),
            signature,
        }
    }
}
