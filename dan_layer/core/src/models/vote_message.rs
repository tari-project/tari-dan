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
use tari_core::ValidatorNodeMmr;
use tari_dan_common_types::{
    hashing::tari_hasher,
    vn_mmr_node_hash,
    QuorumDecision,
    QuorumRejectReason,
    ShardId,
    ShardPledgeCollection,
    TreeNodeHash,
    ValidatorMetadata,
};
use tari_mmr::MerkleProof;

use crate::{services::SigningService, workers::hotstuff_error::HotStuffError, TariDanCoreHashDomain};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VoteMessage {
    local_node_hash: TreeNodeHash,
    decision: QuorumDecision,
    all_shard_pledges: ShardPledgeCollection,
    validator_metadata: Option<ValidatorMetadata>,
}

impl VoteMessage {
    pub fn new(local_node_hash: TreeNodeHash, decision: QuorumDecision, shard_pledges: ShardPledgeCollection) -> Self {
        Self {
            local_node_hash,
            decision,
            all_shard_pledges: shard_pledges,
            validator_metadata: None,
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
    ) -> Self {
        Self {
            local_node_hash,
            decision,
            all_shard_pledges,
            validator_metadata: Some(validator_metadata),
        }
    }

    pub fn sign_vote<TSigningService: SigningService>(
        &mut self,
        signing_service: &TSigningService,
        shard_id: ShardId,
        vn_mmr: &ValidatorNodeMmr,
    ) -> Result<(), HotStuffError> {
        // calculate the signature
        let challenge = self.construct_challenge();
        let signature = signing_service.sign(&*challenge).ok_or(HotStuffError::FailedToSignQc)?;
        // construct the merkle proof for the inclusion of the VN's public key in the epoch
        let leaf_index = vn_mmr
            .find_leaf_index(&*vn_mmr_node_hash(signing_service.public_key(), &shard_id))
            .expect("Unexpected Merkle Mountain Range error")
            .ok_or(HotStuffError::ValidatorNodeNotIncludedInMmr)?;
        let merkle_proof =
            MerkleProof::for_leaf_node(vn_mmr, leaf_index as usize).expect("Merkle proof generation failed");

        let hash = vn_mmr_node_hash(signing_service.public_key(), &shard_id);
        let root = vn_mmr.get_merkle_root().unwrap();
        let idx = vn_mmr.find_leaf_index(&*hash).unwrap();
        // TODO: remove
        if let Err(err) =
            merkle_proof.verify_leaf::<tari_core::ValidatorNodeMmrHasherBlake256>(&root, &*hash, leaf_index as usize)
        {
            log::warn!(
                target: "tari::dan_layer::votemessage",
                "Merkle proof verification failed for validator node {:?} at index {:?} with error: {}",
                hash,
                idx,
                err
            );
        }

        let validator_metadata = ValidatorMetadata::new(
            signing_service.public_key().clone(),
            shard_id,
            signature,
            merkle_proof,
            leaf_index.into(),
        );

        self.validator_metadata = Some(validator_metadata);
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
}
