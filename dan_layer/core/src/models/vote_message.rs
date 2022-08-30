use digest::{Digest, FixedOutput};
use tari_common_types::types::FixedHash;
use tari_crypto::hash::blake2::Blake256;

use crate::models::{ObjectPledge, QuorumDecision, ShardId, TreeNodeHash, ValidatorSignature};

#[derive(Debug, Clone)]
pub struct VoteMessage {
    local_node_hash: TreeNodeHash,
    shard: ShardId,
    decision: QuorumDecision,
    other_shard_nodes: Vec<(ShardId, TreeNodeHash, Vec<ObjectPledge>)>,
    signature: Option<ValidatorSignature>,
}

impl VoteMessage {
    pub fn new(
        local_node_hash: TreeNodeHash,
        shard: ShardId,
        decision: QuorumDecision,
        mut other_shard_nodes: Vec<(ShardId, TreeNodeHash, Vec<ObjectPledge>)>,
    ) -> Self {
        other_shard_nodes.sort_by(|a, b| a.0.cmp(&b.0));

        Self {
            local_node_hash,
            shard,
            decision,
            other_shard_nodes,
            signature: None,
        }
    }

    pub fn sign(&mut self) {
        // TODO: better signature
        self.signature = Some(ValidatorSignature::from_bytes(&[9u8; 32]))
    }

    pub fn signature(&self) -> &ValidatorSignature {
        self.signature.as_ref().unwrap()
    }

    pub fn get_all_nodes_hash(&self) -> FixedHash {
        let mut result = Blake256::new().chain(&[self.decision.as_u8()]);
        // data must already be sorted
        for (shard, hash, pledges) in &self.other_shard_nodes {
            result = result
                .chain(shard.0.to_le_bytes())
                .chain(hash.as_bytes())
                .chain((pledges.len() as u32).to_le_bytes());

            for p in pledges {
                result = result.chain(p.object_id.0.to_le_bytes())
            }
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

    pub fn other_shard_nodes(&self) -> &Vec<(ShardId, TreeNodeHash, Vec<ObjectPledge>)> {
        &self.other_shard_nodes
    }
}
