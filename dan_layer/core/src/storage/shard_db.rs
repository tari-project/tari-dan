use std::collections::HashMap;

use crate::{
    models::{
        vote_message::VoteMessage,
        HotStuffTreeNode,
        NodeHeight,
        ObjectId,
        ObjectPledge,
        Payload,
        PayloadId,
        QuorumCertificate,
        ShardId,
        SubstateChange,
        SubstateState,
        TreeNodeHash,
    },
    services::infrastructure_services::NodeAddressable,
};

pub struct ShardDb<TAddr: NodeAddressable, TPayload: Payload> {
    // replica data
    shard_high_qcs: HashMap<ShardId, QuorumCertificate>,
    // pace maker data
    shard_leaf_nodes: HashMap<ShardId, (TreeNodeHash, NodeHeight)>,
    last_voted_heights: HashMap<ShardId, NodeHeight>,
    lock_node_and_heights: HashMap<ShardId, (TreeNodeHash, NodeHeight)>,
    votes: HashMap<(TreeNodeHash, ShardId), Vec<(TAddr, VoteMessage)>>,
    nodes: HashMap<TreeNodeHash, HotStuffTreeNode<TAddr>>,
    last_executed_height: HashMap<ShardId, NodeHeight>,
    payloads: HashMap<PayloadId, TPayload>,
    payload_votes: HashMap<PayloadId, HashMap<NodeHeight, HashMap<ShardId, HotStuffTreeNode<TAddr>>>>,
    objects: HashMap<ShardId, HashMap<ObjectId, (SubstateState, Option<ObjectPledge>)>>,
}

impl<TAddr: NodeAddressable, TPayload: Payload> ShardDb<TAddr, TPayload> {
    pub fn new() -> Self {
        ShardDb {
            shard_high_qcs: HashMap::new(),
            shard_leaf_nodes: HashMap::new(),
            last_voted_heights: HashMap::new(),
            lock_node_and_heights: HashMap::new(),
            votes: HashMap::new(),
            nodes: HashMap::new(),
            last_executed_height: HashMap::new(),
            payloads: HashMap::new(),
            payload_votes: HashMap::new(),
            objects: HashMap::new(),
        }
    }

    pub fn get_high_qc_for(&self, shard: ShardId) -> QuorumCertificate {
        if let Some(qc) = self.shard_high_qcs.get(&shard) {
            qc.clone()
        } else {
            QuorumCertificate::genesis()
        }
    }

    pub fn update_high_qc(&mut self, qc: QuorumCertificate) {
        let entry = self.shard_high_qcs.entry(qc.shard()).or_insert(qc.clone());
        if qc.local_node_height() > entry.local_node_height() {
            *entry = qc.clone();
            self.shard_leaf_nodes
                .entry(qc.shard())
                .and_modify(|e| *e = (qc.local_node_hash().clone(), qc.local_node_height()))
                .or_insert((qc.local_node_hash().clone(), qc.local_node_height()));
        }
    }

    pub fn get_leaf_node(&self, shard: ShardId) -> (TreeNodeHash, NodeHeight) {
        if let Some(leaf) = self.shard_leaf_nodes.get(&shard) {
            leaf.clone()
        } else {
            (TreeNodeHash::zero(), NodeHeight(0))
        }
    }

    pub fn update_leaf_node(&mut self, shard: ShardId, node: TreeNodeHash, height: NodeHeight) -> Result<(), String> {
        let leaf = self.shard_leaf_nodes.entry(shard).or_insert((node, height));
        *leaf = (node, height);
        Ok(())
    }

    pub fn get_last_voted_height(&self, shard: ShardId) -> NodeHeight {
        self.last_voted_heights.get(&shard).map(|e| *e).unwrap_or(NodeHeight(0))
    }

    pub fn set_last_voted_height(&mut self, shard: ShardId, height: NodeHeight) {
        let entry = self.last_voted_heights.entry(shard).or_insert(height);
        *entry = height;
    }

    pub fn get_locked_node_hash_and_height(&self, shard: ShardId) -> (TreeNodeHash, NodeHeight) {
        self.lock_node_and_heights
            .get(&shard)
            .unwrap_or(&(TreeNodeHash::zero(), NodeHeight(0)))
            .clone()
    }

    pub fn set_locked(&mut self, shard: ShardId, node_hash: TreeNodeHash, node_height: NodeHeight) {
        self.lock_node_and_heights
            .entry(shard)
            .and_modify(|e| *e = (node_hash, node_height));
    }

    pub fn has_vote_for(&self, from: &TAddr, node_hash: TreeNodeHash, shard: ShardId) -> bool {
        if let Some(sigs) = self.votes.get(&(node_hash, shard)) {
            sigs.iter().any(|(f, _)| f == from)
        } else {
            false
        }
    }

    pub fn save_received_vote_for(
        &mut self,
        from: TAddr,
        node_hash: TreeNodeHash,
        shard: ShardId,
        vote_message: VoteMessage,
    ) -> usize {
        let entry = self.votes.entry((node_hash, shard)).or_insert(vec![]);
        entry.push((from, vote_message));
        entry.len()
    }

    pub fn get_received_votes_for(&self, node_hash: TreeNodeHash, shard: ShardId) -> Vec<VoteMessage> {
        self.votes
            .get(&(node_hash, shard))
            .map(|v| v.iter().map(|s| s.1.clone()).collect())
            .unwrap_or(vec![])
    }

    pub fn save_payload_vote(
        &mut self,
        shard: ShardId,
        payload: PayloadId,
        payload_height: NodeHeight,
        node: HotStuffTreeNode<TAddr>,
    ) {
        let payload_entry = self.payload_votes.entry(payload).or_insert(HashMap::new());
        let height_entry = payload_entry.entry(payload_height).or_insert(HashMap::new());
        height_entry
            .entry(shard)
            .and_modify(|e| *e = node.clone())
            .or_insert(node);
    }

    pub fn get_payload_vote(
        &self,
        payload: PayloadId,
        payload_height: NodeHeight,
        shard: ShardId,
    ) -> Option<HotStuffTreeNode<TAddr>> {
        self.payload_votes
            .get(&payload)
            .and_then(|pv| pv.get(&payload_height))
            .and_then(|ph| ph.get(&shard).cloned())
    }

    pub fn save_node(&mut self, node: HotStuffTreeNode<TAddr>) {
        self.nodes.entry(node.hash().clone()).or_insert(node.clone());
    }

    pub fn node(&self, node_hash: &TreeNodeHash) -> Option<HotStuffTreeNode<TAddr>> {
        if node_hash == &TreeNodeHash::zero() {
            Some(HotStuffTreeNode::genesis())
        } else {
            self.nodes.get(node_hash).cloned()
        }
    }

    pub fn set_last_executed_height(&mut self, shard: ShardId, height: NodeHeight) {
        self.last_executed_height.entry(shard).and_modify(|e| *e = height);
    }

    pub fn get_last_executed_height(&self, shard: ShardId) -> NodeHeight {
        self.last_executed_height
            .get(&shard)
            .map(|s| *s)
            .unwrap_or(NodeHeight(0))
    }

    pub fn get_payload(&self, payload_id: &PayloadId) -> Option<&TPayload> {
        self.payloads.get(payload_id)
    }

    pub fn set_payload(&mut self, payload: TPayload) {
        let payload_id = payload.to_id();
        self.payloads.entry(payload_id).or_insert(payload);
    }

    pub fn pledge_object(
        &mut self,
        shard: ShardId,
        object: ObjectId,
        change: SubstateChange,
        payload: PayloadId,
        current_height: NodeHeight,
    ) -> ObjectPledge {
        let shard_data = self.objects.entry(shard).or_insert(HashMap::new());
        let entry = shard_data.entry(object).or_insert((SubstateState::DoesNotExist, None));
        if let Some(existing_pledge) = &entry.1 {
            if existing_pledge.pledged_until < current_height {
                return existing_pledge.clone();
            }
        }

        let pledge = ObjectPledge {
            object_id: object,
            current_state: entry.0.clone(),
            pledged_to_payload: payload,
            pledged_until: current_height + NodeHeight(4),
        };
        entry.1 = Some(pledge.clone());
        pledge
    }
}
