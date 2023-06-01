//  Copyright 2022. The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::io;

use log::*;
use serde::{Deserialize, Serialize};
use tari_common_types::types::FixedHash;
use tari_core::{ValidatorNodeBMT, ValidatorNodeBmtHasherBlake256};
use tari_dan_common_types::{
    hashing::tari_hasher,
    vn_bmt_node_hash,
    QuorumDecision,
    QuorumRejectReason,
    ShardId,
    ShardPledgeCollection,
    TreeNodeHash,
    ValidatorMetadata,
};
use tari_mmr::BalancedBinaryMerkleProof;
use tari_utilities::hex::Hex;

use crate::{services::SigningService, workers::hotstuff_error::HotStuffError, TariDanCoreHashDomain};

const LOG_TARGET: &str = "tari::validator_node::models::vote_message";

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VoteMessage {
    local_node_hash: TreeNodeHash,
    decision: QuorumDecision,
    all_shard_pledges: ShardPledgeCollection,
    validator_metadata: Option<ValidatorMetadata>,
    merkle_proof: Option<BalancedBinaryMerkleProof<ValidatorNodeBmtHasherBlake256>>,
    node_hash: FixedHash,
    shard_id: ShardId,
}

impl VoteMessage {
    pub fn new(
        local_node_hash: TreeNodeHash,
        decision: QuorumDecision,
        shard_pledges: ShardPledgeCollection,
        shard_id: ShardId,
    ) -> Self {
        Self {
            local_node_hash,
            decision,
            all_shard_pledges: shard_pledges,
            validator_metadata: None,
            merkle_proof: None,
            node_hash: FixedHash::zero(),
            shard_id,
        }
    }

    pub fn shard(&self) -> ShardId {
        self.shard_id
    }

    pub fn accept(local_node_hash: TreeNodeHash, shard_pledges: ShardPledgeCollection, shard_id: ShardId) -> Self {
        Self::new(local_node_hash, QuorumDecision::Accept, shard_pledges, shard_id)
    }

    pub fn reject(
        local_node_hash: TreeNodeHash,
        shard_pledges: ShardPledgeCollection,
        reason: QuorumRejectReason,
        shard_id: ShardId,
    ) -> Self {
        let decision = QuorumDecision::Reject(reason);
        Self::new(local_node_hash, decision, shard_pledges, shard_id)
    }

    pub fn with_validator_metadata(
        local_node_hash: TreeNodeHash,
        decision: QuorumDecision,
        all_shard_pledges: ShardPledgeCollection,
        validator_metadata: ValidatorMetadata,
        merkle_proof: Option<BalancedBinaryMerkleProof<ValidatorNodeBmtHasherBlake256>>,
        node_hash: FixedHash,
        shard_id: ShardId,
    ) -> Self {
        Self {
            local_node_hash,
            decision,
            all_shard_pledges,
            validator_metadata: Some(validator_metadata),
            merkle_proof,
            node_hash,
            shard_id,
        }
    }

    pub fn sign_vote<TSigningService: SigningService>(
        &mut self,
        signing_service: &TSigningService,
        shard_id: ShardId,
        vn_bmt: &ValidatorNodeBMT,
    ) -> Result<(), HotStuffError> {
        // calculate the signature
        let challenge = self.construct_challenge();
        let signature = signing_service.sign(&*challenge).ok_or(HotStuffError::FailedToSignQc)?;
        // construct the merkle proof for the inclusion of the VN's public key in the epoch
        let node_hash = vn_bmt_node_hash(signing_service.public_key(), &shard_id);
        debug!(
            target: LOG_TARGET,
            "[sign_vote] bmt_node_hash={}, public_key={}, shard_id={}",
            node_hash.to_hex(),
            signing_service.public_key(),
            shard_id,
        );
        let leaf_index = vn_bmt
            .find_leaf_index_for_hash(&node_hash.to_vec())
            .map_err(|_| HotStuffError::ValidatorNodeNotIncludedInBMT)?;
        let merkle_proof = BalancedBinaryMerkleProof::generate_proof(vn_bmt, leaf_index as usize)
            .map_err(|_| HotStuffError::FailedToGenerateMerkleProof)?;

        let root = vn_bmt.get_merkle_root();
        let idx = vn_bmt
            .find_leaf_index_for_hash(&node_hash.to_vec())
            .map_err(|_| HotStuffError::ValidatorNodeNotIncludedInBMT)?;
        // TODO: remove
        if !merkle_proof.verify(&root, node_hash.to_vec()) {
            log::warn!(
                target: "tari::dan_layer::votemessage",
                "Merkle proof verification failed for validator node {} at index {:?}",
                node_hash.to_hex(),
                idx,
            );
        }

        let validator_metadata = ValidatorMetadata::new(signing_service.public_key().clone(), shard_id, signature);

        self.validator_metadata = Some(validator_metadata);
        self.merkle_proof = Some(merkle_proof);
        self.node_hash = node_hash;
        Ok(())
    }

    pub fn construct_challenge(&self) -> FixedHash {
        tari_hasher::<TariDanCoreHashDomain>("vote_message")
            .chain(self.local_node_hash.as_bytes())
            .chain(&[self.decision.as_u8()])
            .chain(self.all_shard_pledges())
            .result()
    }

    pub fn validator_metadata(&self) -> &ValidatorMetadata {
        self.validator_metadata.as_ref().unwrap()
    }

    pub fn local_node_hash(&self) -> TreeNodeHash {
        self.local_node_hash
    }

    pub fn decision(&self) -> QuorumDecision {
        self.decision
    }

    pub fn all_shard_pledges(&self) -> &ShardPledgeCollection {
        &self.all_shard_pledges
    }

    pub fn merkle_proof(&self) -> Option<BalancedBinaryMerkleProof<ValidatorNodeBmtHasherBlake256>> {
        self.merkle_proof.clone()
    }

    pub fn encode_merkle_proof(&self) -> Vec<u8> {
        bincode::serialize(&self.merkle_proof).unwrap()
    }

    pub fn decode_merkle_proof(
        bytes: &[u8],
    ) -> Result<Option<BalancedBinaryMerkleProof<ValidatorNodeBmtHasherBlake256>>, io::Error> {
        // Map to an io error because borsh uses that
        bincode::deserialize(bytes).map_err(|e| io::Error::new(io::ErrorKind::Other, e))
    }

    pub fn node_hash(&self) -> FixedHash {
        self.node_hash
    }
}
