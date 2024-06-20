//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_common_types::types::{FixedHash, PublicKey};
use tari_dan_common_types::hashing::vote_signature_hasher;
use tari_dan_storage::consensus_models::{BlockId, QuorumDecision, ValidatorSchnorrSignature, ValidatorSignature};

pub trait ValidatorSignatureService {
    fn sign<M: AsRef<[u8]>>(&self, message: M) -> ValidatorSchnorrSignature;

    fn public_key(&self) -> &PublicKey;
}

pub trait VoteSignatureService: ValidatorSignatureService {
    fn create_challenge(&self, block_id: &BlockId, decision: &QuorumDecision) -> FixedHash {
        vote_signature_hasher().chain(block_id).chain(decision).result()
    }

    fn sign_vote(&self, block_id: &BlockId, decision: &QuorumDecision) -> ValidatorSignature {
        let challenge = self.create_challenge(block_id, decision);
        let signature = self.sign(challenge);
        ValidatorSignature::new(self.public_key().clone(), signature)
    }

    fn verify(&self, signature: &ValidatorSignature, block_id: &BlockId, decision: &QuorumDecision) -> bool;
}
