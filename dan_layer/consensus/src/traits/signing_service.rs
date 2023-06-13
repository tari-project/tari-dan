//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_common_types::types::{FixedHash, PublicKey, Signature};
use tari_dan_common_types::Epoch;
use tari_dan_storage::consensus_models::BlockId;

use crate::messages::QuorumDecision;

pub trait SigningService {
    fn sign(&self, challenge: &[u8]) -> Signature;
    fn verify(&self, signature: &Signature, challenge: &[u8]) -> bool;
    fn verify_for_public_key(&self, public_key: &PublicKey, signature: &Signature, challenge: &[u8]) -> bool;
    fn public_key(&self) -> &PublicKey;
}

pub trait VoteSigningService: SigningService {
    fn create_challenge(epoch: Epoch, block_id: &BlockId, decision: QuorumDecision) -> FixedHash;
    fn sign_vote(&self, epoch: Epoch, block_id: &BlockId, decision: QuorumDecision) -> Signature {
        let challenge = Self::create_challenge(epoch, block_id, decision);
        self.sign(&*challenge)
    }
}
