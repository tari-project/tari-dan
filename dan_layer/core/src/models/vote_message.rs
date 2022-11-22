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

use borsh::BorshSerialize;
use digest::{Digest, FixedOutput};
use serde::{Deserialize, Serialize};
use tari_common_types::types::{FixedHash, PrivateKey, PublicKey};
use tari_core::{consensus::DomainSeparatedConsensusHasher, transactions::TransactionHashDomain, ValidatorNodeMmr};
use tari_crypto::hash::blake2::Blake256;
use tari_dan_common_types::ShardId;
use tari_dan_engine::crypto::create_key_pair;
use tari_engine_types::commit_result::RejectReason;

use super::quorum_certificate::QuorumRejectReason;
use crate::models::{QuorumDecision, ShardVote, TreeNodeHash, ValidatorMetadata};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VoteMessage {
    local_node_hash: TreeNodeHash,
    shard: ShardId,
    decision: QuorumDecision,
    all_shard_nodes: Vec<ShardVote>,
    validator_metadata: Option<ValidatorMetadata>,
}

impl VoteMessage {
    pub fn new(
        local_node_hash: TreeNodeHash,
        shard: ShardId,
        decision: QuorumDecision,
        mut all_shard_nodes: Vec<ShardVote>,
    ) -> Self {
        all_shard_nodes.sort_by(|a, b| a.shard_id.cmp(&b.shard_id));

        Self {
            local_node_hash,
            shard,
            decision,
            all_shard_nodes,
            validator_metadata: None,
        }
    }

    pub fn accept(local_node_hash: TreeNodeHash, shard: ShardId, all_shard_nodes: Vec<ShardVote>) -> Self {
        Self::new(local_node_hash, shard, QuorumDecision::Accept, all_shard_nodes)
    }

    pub fn reject(
        local_node_hash: TreeNodeHash,
        shard: ShardId,
        all_shard_nodes: Vec<ShardVote>,
        reason: &RejectReason,
    ) -> Self {
        let quorum_reject_reason = match reason {
            RejectReason::ShardNotPledged(_) => QuorumRejectReason::ShardNotPledged,
            RejectReason::ExecutionFailure(_) => QuorumRejectReason::ExecutionFailure,
        };
        let decision = QuorumDecision::Reject(quorum_reject_reason);

        Self::new(local_node_hash, shard, decision, all_shard_nodes)
    }

    pub fn with_validator_metadata(
        local_node_hash: TreeNodeHash,
        shard: ShardId,
        decision: QuorumDecision,
        mut all_shard_nodes: Vec<ShardVote>,
        validator_metadata: ValidatorMetadata,
    ) -> Self {
        all_shard_nodes.sort_by(|a, b| a.shard_id.cmp(&b.shard_id));

        Self {
            local_node_hash,
            shard,
            decision,
            all_shard_nodes,
            validator_metadata: Some(validator_metadata),
        }
    }

    pub fn set_metadata(
        &mut self,
        public_key: &PublicKey,
        secret_key: &PrivateKey,
        vn_mmr: &ValidatorNodeMmr,
        vn_mmr_leaf_index: u64,
    ) {
        let (secret_nonce, public_nonce) = create_key_pair();
        let challenge = self.construct_challenge(public_key, &public_nonce);
        let validator_metadata = ValidatorMetadata::new(
            public_key,
            secret_key,
            secret_nonce,
            &*challenge,
            vn_mmr,
            vn_mmr_leaf_index,
        );

        self.validator_metadata = Some(validator_metadata);
    }

    pub fn construct_challenge(&self, public_key: &PublicKey, public_nonce: &PublicKey) -> FixedHash {
        // TODO remove this when we switch from consensus to Borsh. Hasher will do serialize instead of consesus so no
        // need to serialize it here.
        let mut serialized_all_shard_nodes = Vec::new();
        BorshSerialize::serialize(self.all_shard_nodes(), &mut serialized_all_shard_nodes).unwrap();
        DomainSeparatedConsensusHasher::<TransactionHashDomain>::new("vote_message")
            .chain(public_key)
            .chain(public_nonce)
            .chain(&self.local_node_hash.as_bytes())
            .chain(&self.shard.as_bytes())
            .chain(&[self.decision.as_u8()])
            .chain(&serialized_all_shard_nodes)
            .finalize()
            .into()
    }

    pub fn validator_metadata(&self) -> &ValidatorMetadata {
        self.validator_metadata.as_ref().unwrap()
    }

    pub fn get_all_nodes_hash(&self) -> FixedHash {
        let mut result = Blake256::new().chain([self.decision.as_u8()]);
        // data must already be sorted
        for ShardVote {
            shard_id,
            node_hash,
            pledge,
        } in &self.all_shard_nodes
        {
            result = result
                .chain(shard_id.0)
                .chain(node_hash.as_bytes())
                // TODO: borsh serialize pledge
                .chain(pledge.as_ref().map(|p| p.shard_id.0).unwrap_or_default());
        }
        result.finalize_fixed().into()
    }

    pub fn local_node_hash(&self) -> TreeNodeHash {
        self.local_node_hash
    }

    pub fn shard(&self) -> ShardId {
        self.shard
    }

    pub fn decision(&self) -> QuorumDecision {
        self.decision
    }

    pub fn all_shard_nodes(&self) -> &Vec<ShardVote> {
        &self.all_shard_nodes
    }
}
