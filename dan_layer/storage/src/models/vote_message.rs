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

use serde::{Deserialize, Serialize};
use tari_common_types::types::FixedHash;
use tari_core::ValidatorNodeBmtHasherBlake256;
use tari_dan_common_types::{
    QuorumDecision,
    QuorumRejectReason,
    ShardPledgeCollection,
    TreeNodeHash,
    ValidatorMetadata,
};
use tari_mmr::BalancedBinaryMerkleProof;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VoteMessage {
    local_node_hash: TreeNodeHash,
    decision: QuorumDecision,
    all_shard_pledges: ShardPledgeCollection,
    validator_metadata: Option<ValidatorMetadata>,
    merkle_proof: Option<BalancedBinaryMerkleProof<ValidatorNodeBmtHasherBlake256>>,
    node_hash: FixedHash,
}

impl VoteMessage {
    pub fn new(local_node_hash: TreeNodeHash, decision: QuorumDecision, shard_pledges: ShardPledgeCollection) -> Self {
        Self {
            local_node_hash,
            decision,
            all_shard_pledges: shard_pledges,
            validator_metadata: None,
            merkle_proof: None,
            node_hash: FixedHash::zero(),
        }
    }

    pub fn accept(local_node_hash: TreeNodeHash, shard_pledges: ShardPledgeCollection) -> Self {
        Self::new(local_node_hash, QuorumDecision::Accept, shard_pledges)
    }

    pub fn reject(
        local_node_hash: TreeNodeHash,
        shard_pledges: ShardPledgeCollection,
        reason: QuorumRejectReason,
    ) -> Self {
        let decision = QuorumDecision::Reject(reason);
        Self::new(local_node_hash, decision, shard_pledges)
    }

    pub fn with_validator_metadata(
        local_node_hash: TreeNodeHash,
        decision: QuorumDecision,
        all_shard_pledges: ShardPledgeCollection,
        validator_metadata: ValidatorMetadata,
        merkle_proof: Option<BalancedBinaryMerkleProof<ValidatorNodeBmtHasherBlake256>>,
        node_hash: FixedHash,
    ) -> Self {
        Self {
            local_node_hash,
            decision,
            all_shard_pledges,
            validator_metadata: Some(validator_metadata),
            merkle_proof,
            node_hash,
        }
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

    pub fn node_hash(&self) -> FixedHash {
        self.node_hash
    }

    pub fn set_node_hash(&mut self, node_hash: FixedHash) -> &mut Self {
        self.node_hash = node_hash;
        self
    }

    pub fn set_merkle_proof(
        &mut self,
        merkle_proof: BalancedBinaryMerkleProof<ValidatorNodeBmtHasherBlake256>,
    ) -> &mut Self {
        self.merkle_proof = Some(merkle_proof);
        self
    }

    pub fn set_validator_metadata(&mut self, metadata: ValidatorMetadata) -> &mut Self {
        self.validator_metadata = Some(metadata);
        self
    }
}
