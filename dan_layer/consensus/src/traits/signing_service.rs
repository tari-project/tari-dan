//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_common_types::types::FixedHash;
use tari_dan_common_types::{hashing::vote_signature_hasher, NodeAddressable};
use tari_dan_storage::consensus_models::{BlockId, QuorumDecision, ValidatorSchnorrSignature, ValidatorSignature};

pub trait ValidatorSignatureService<TAddr> {
    fn sign<M: AsRef<[u8]>>(&self, message: M) -> ValidatorSchnorrSignature;

    fn public_key(&self) -> &TAddr;
}

pub trait VoteSignatureService<TAddr: NodeAddressable>: ValidatorSignatureService<TAddr> {
    fn create_challenge(
        &self,
        voter_leaf_hash: &FixedHash,
        block_id: &BlockId,
        decision: &QuorumDecision,
    ) -> FixedHash {
        vote_signature_hasher()
            .chain(voter_leaf_hash)
            .chain(block_id)
            .chain(decision)
            .result()
    }

    fn sign_vote(
        &self,
        leaf_hash: &FixedHash,
        block_id: &BlockId,
        decision: &QuorumDecision,
    ) -> ValidatorSignature<TAddr> {
        let challenge = self.create_challenge(leaf_hash, block_id, decision);
        let signature = self.sign(challenge);
        ValidatorSignature::new(self.public_key().clone(), signature)
    }

    fn verify(
        &self,
        signature: &ValidatorSignature<TAddr>,
        leaf_hash: &FixedHash,
        block_id: &BlockId,
        decision: &QuorumDecision,
    ) -> bool;
}
