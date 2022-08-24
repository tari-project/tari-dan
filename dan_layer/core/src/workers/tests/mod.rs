use std::{
    collections::HashMap,
    ops::{Index, Range, RangeBounds},
    time::Duration,
};

use async_recursion::async_recursion;
use clap::command;
use tari_common_types::types::FixedHash;
use tari_dan_engine::instruction::Instruction;
use tokio::{
    sync::mpsc::{channel, Receiver, Sender},
    task::JoinHandle,
    time::timeout,
};

use crate::{
    models::{
        Committee,
        Epoch,
        HotStuffMessage,
        HotStuffMessageType,
        HotStuffMessageType::Commit,
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
        TariDanPayload,
        TreeNodeHash,
        ValidatorSignature,
        ViewId,
    },
    services::infrastructure_services::NodeAddressable,
};

pub trait Consensus<TPayload: Payload> {
    fn execute_transaction(
        &mut self,
        payload: TPayload,
        inputs: Vec<ObjectPledge>,
        outputs: Vec<ObjectPledge>,
    ) -> Result<(), String>;
}

pub struct ShardDb<TAddr: NodeAddressable, TPayload: Payload> {
    // replica data
    shard_high_qcs: HashMap<ShardId, QuorumCertificate>,
    // pace maker data
    shard_leaf_nodes: HashMap<ShardId, (TreeNodeHash, NodeHeight)>,
    last_voted_heights: HashMap<ShardId, NodeHeight>,
    lock_node_and_heights: HashMap<ShardId, (TreeNodeHash, NodeHeight)>,
    votes: HashMap<(TreeNodeHash, ShardId), Vec<(TAddr, ValidatorSignature)>>,
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

    pub fn save_vote_for(
        &mut self,
        from: TAddr,
        node_hash: TreeNodeHash,
        shard: ShardId,
        signature: ValidatorSignature,
    ) -> usize {
        let entry = self.votes.entry((node_hash, shard)).or_insert(vec![]);
        entry.push((from, signature));
        entry.len()
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
        dbg!(&self.payload_votes);
        self.payload_votes
            .get(&payload)
            .and_then(|pv| pv.get(&payload_height))
            .and_then(|ph| ph.get(&shard).cloned())
    }

    pub fn get_signatures_for(&self, node_hash: TreeNodeHash, shard: ShardId) -> Vec<(TAddr, ValidatorSignature)> {
        if let Some(sigs) = self.votes.get(&(node_hash, shard)) {
            sigs.clone()
        } else {
            vec![]
        }
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

#[derive(Debug, Clone)]
pub struct VoteMessage {
    pub local_node_hash: TreeNodeHash,
    pub shard: ShardId,
    pub other_shard_nodes: Vec<(ShardId, TreeNodeHash)>,
    pub signature: ValidatorSignature,
}

pub trait LeaderStrategy<TAddr: NodeAddressable, TPayload> {
    fn calculate_leader(&self, committee: &Committee<TAddr>, payload: PayloadId, shard: ShardId, round: u32) -> u32;
    fn is_leader(
        &self,
        node: &TAddr,
        committee: &Committee<TAddr>,
        payload: PayloadId,
        shard: ShardId,
        round: u32,
    ) -> bool {
        let position = self.calculate_leader(committee, payload, shard, round);
        if let Some(index) = committee.members.iter().position(|m| m == node) {
            position == index as u32
        } else {
            false
        }
    }

    fn get_leader<'a, 'b>(
        &'a self,
        committee: &'b Committee<TAddr>,
        payload: PayloadId,
        shard: ShardId,
        round: u32,
    ) -> &'b TAddr {
        let index = self.calculate_leader(committee, payload, shard, round);
        committee.members.get(index as usize).unwrap()
    }
}

pub struct AlwaysFirstLeader {}

impl<TAddr: NodeAddressable, TPayload> LeaderStrategy<TAddr, TPayload> for AlwaysFirstLeader {
    fn calculate_leader(&self, committee: &Committee<TAddr>, payload: PayloadId, shard: ShardId, round: u32) -> u32 {
        0
    }
}

pub trait EpochManager<TAddr: NodeAddressable>: Clone {
    fn current_epoch(&self) -> Epoch;
    fn is_epoch_valid(&self, epoch: Epoch) -> bool;
    fn get_committees(
        &self,
        epoch: Epoch,
        shards: &[ShardId],
    ) -> Result<Vec<(ShardId, Option<Committee<TAddr>>)>, String>;
    fn get_committee(&self, epoch: Epoch, shard: ShardId) -> Result<Committee<TAddr>, String>;
    fn get_shards(&self, epoch: Epoch, addr: &TAddr, available_shards: &[ShardId]) -> Result<Vec<ShardId>, String>;
}

#[derive(Debug, Clone)]
pub struct RangeEpochManager<TAddr: NodeAddressable> {
    current_epoch: Epoch,
    epochs: HashMap<Epoch, Vec<(Range<ShardId>, Committee<TAddr>)>>,
}

impl<TAddr: NodeAddressable> RangeEpochManager<TAddr> {
    pub fn new(current: Range<ShardId>, committee: Vec<TAddr>) -> Self {
        let mut epochs = HashMap::new();
        epochs.insert(Epoch(0), vec![(current, Committee::new(committee))]);
        Self {
            current_epoch: Epoch(0),
            epochs,
        }
    }

    pub fn new_with_multiple(ranges: &[(Range<ShardId>, Vec<TAddr>)]) -> Self {
        let mut epochs = HashMap::new();
        epochs.insert(
            Epoch(0),
            ranges
                .iter()
                .map(|r| (r.0.clone(), Committee::new(r.1.clone())))
                .collect(),
        );
        Self {
            current_epoch: Epoch(0),
            epochs,
        }
    }
}

impl<TAddr: NodeAddressable> EpochManager<TAddr> for RangeEpochManager<TAddr> {
    fn current_epoch(&self) -> Epoch {
        self.current_epoch
    }

    fn is_epoch_valid(&self, epoch: Epoch) -> bool {
        self.current_epoch == epoch
    }

    fn get_committees(
        &self,
        epoch: Epoch,
        shards: &[ShardId],
    ) -> Result<Vec<(ShardId, Option<Committee<TAddr>>)>, String> {
        let epoch = self.epochs.get(&epoch).ok_or("No value for that epoch".to_string())?;
        let mut result = vec![];
        for shard in shards {
            let mut found_committee = None;
            for (range, committee) in epoch {
                if range.contains(shard) {
                    found_committee = Some(committee.clone());
                    break;
                }
            }
            result.push((*shard, found_committee.clone()));
        }

        Ok(result)
    }

    fn get_committee(&self, epoch: Epoch, shard: ShardId) -> Result<Committee<TAddr>, String> {
        let epoch = self.epochs.get(&epoch).ok_or("No value for that epoch".to_string())?;
        for (range, committee) in epoch {
            if range.contains(&shard) {
                return Ok(committee.clone());
            }
        }
        Err("Could not find a committee for that shard".to_string())
    }

    fn get_shards(&self, epoch: Epoch, addr: &TAddr, available_shards: &[ShardId]) -> Result<Vec<ShardId>, String> {
        let epoch = self.epochs.get(&epoch).ok_or("No value for that epoch".to_string())?;
        let mut result = vec![];
        for (range, committee) in epoch {
            for shard in available_shards {
                if range.contains(shard) {
                    if committee.contains(addr) {
                        result.push(*shard);
                    }
                }
            }
        }

        Ok(result)
    }
}

pub struct HotStuffWaiter<
    TPayload: Payload,
    TAddr: NodeAddressable,
    TLeaderStrategy: LeaderStrategy<TAddr, TPayload>,
    TEpochManager: EpochManager<TAddr>,
> {
    identity: TAddr,
    leader_strategy: TLeaderStrategy,
    epoch_manager: TEpochManager,
    rx_new: Receiver<(TPayload, ShardId)>,
    rx_hs_message: Receiver<(TAddr, HotStuffMessage<TPayload, TAddr>)>,
    rx_votes: Receiver<(TAddr, VoteMessage)>,
    tx_leader: Sender<HotStuffMessage<TPayload, TAddr>>,
    tx_broadcast: Sender<(HotStuffMessage<TPayload, TAddr>, Vec<TAddr>)>,
    tx_vote_message: Sender<(VoteMessage, TAddr)>,
    // TODO: perhaps change to a service like Payload processor?
    tx_execute: Sender<TPayload>,
    shard_db: ShardDb<TAddr, TPayload>,
}

impl<
        TPayload: Payload + 'static,
        TAddr: NodeAddressable + 'static,
        TLeaderStrategy: LeaderStrategy<TAddr, TPayload> + 'static + Send,
        TEpochManager: EpochManager<TAddr> + 'static + Send,
    > HotStuffWaiter<TPayload, TAddr, TLeaderStrategy, TEpochManager>
{
    pub fn spawn(
        identity: TAddr,
        epoch_manager: TEpochManager,
        leader_strategy: TLeaderStrategy,
        rx_new: Receiver<(TPayload, ShardId)>,
        rx_hs_message: Receiver<(TAddr, HotStuffMessage<TPayload, TAddr>)>,
        rx_votes: Receiver<(TAddr, VoteMessage)>,
        tx_leader: Sender<HotStuffMessage<TPayload, TAddr>>,
        tx_broadcast: Sender<(HotStuffMessage<TPayload, TAddr>, Vec<TAddr>)>,
        tx_vote_message: Sender<(VoteMessage, TAddr)>,
        tx_execute: Sender<TPayload>,
        rx_shutdown: Receiver<()>,
    ) -> JoinHandle<Result<(), String>> {
        tokio::spawn(async move {
            HotStuffWaiter::new(
                identity,
                epoch_manager,
                leader_strategy,
                rx_new,
                rx_hs_message,
                rx_votes,
                tx_leader,
                tx_broadcast,
                tx_vote_message,
                tx_execute,
            )
            .run(rx_shutdown)
            .await
        })
    }

    pub fn new(
        identity: TAddr,
        epoch_manager: TEpochManager,
        leader_strategy: TLeaderStrategy,
        rx_new: Receiver<(TPayload, ShardId)>,
        rx_hs_message: Receiver<(TAddr, HotStuffMessage<TPayload, TAddr>)>,
        rx_votes: Receiver<(TAddr, VoteMessage)>,
        tx_leader: Sender<HotStuffMessage<TPayload, TAddr>>,
        tx_broadcast: Sender<(HotStuffMessage<TPayload, TAddr>, Vec<TAddr>)>,
        tx_vote_message: Sender<(VoteMessage, TAddr)>,
        tx_execute: Sender<TPayload>,
    ) -> Self {
        Self {
            identity,
            epoch_manager,
            leader_strategy,
            rx_new,
            rx_hs_message,
            rx_votes,
            tx_leader,
            tx_broadcast,
            tx_vote_message,
            tx_execute,
            shard_db: ShardDb::new(),
        }
    }

    fn get_highest_qc(&self, shard: ShardId) -> QuorumCertificate {
        self.shard_db.get_high_qc_for(shard)
    }

    // pacemaker
    fn on_receive_new_view(
        &mut self,
        from: TAddr,
        shard: ShardId,
        qc: QuorumCertificate,
        payload: TPayload,
    ) -> Result<(), String> {
        // TODO: Validate who message is from
        self.validate_from_committee(&from, self.epoch_manager.current_epoch(), shard)?;
        self.validate_qc(&qc);
        self.shard_db.update_high_qc(qc);
        self.shard_db.set_payload(payload);
        Ok(())
    }

    // pacemaker
    async fn on_beat(&mut self, shard: ShardId, payload: PayloadId) -> Result<(), String> {
        // TODO: the leader is only known after the leaf is determines
        // TODO: Review if this is correct. The epoch should stay the same for all epochs
        if self.is_leader(payload, shard, self.epoch_manager.current_epoch())? {
            dbg!("I am the leader");
            // if self.current_payload.is_none() {
            // self.current_payload = payload.clone();
            let leaf = self.shard_db.get_leaf_node(shard);
            let node = self.on_propose(leaf.0, leaf.1, shard, payload).await?;
            self.shard_db
                .update_leaf_node(shard, node.hash().clone(), node.height())?;
            // }
        }
        Ok(())
    }

    async fn on_propose(
        &mut self,
        leaf: TreeNodeHash,
        leaf_height: NodeHeight,
        shard: ShardId,
        payload: PayloadId,
    ) -> Result<HotStuffTreeNode<TAddr>, String> {
        dbg!("on propose");
        let qc = self.shard_db.get_high_qc_for(shard);
        let epoch = self.epoch_manager.current_epoch();
        let actual_payload = self
            .shard_db
            .get_payload(&payload)
            .ok_or("Could not find payload".to_string())?;
        let involved_shards = actual_payload.involved_shards().to_vec();
        let members = self
            .epoch_manager
            .get_committees(epoch, &involved_shards)?
            .into_iter()
            .map(|(shard, committee)| committee.map(|c| c.members).unwrap_or_default())
            .flatten()
            .collect();

        let parent = self.shard_db.node(&leaf).ok_or("Could not find leaf".to_string())?;

        let payload_height = if parent.payload() == payload {
            parent.payload_height() + NodeHeight(1)
        } else {
            NodeHeight(0)
        };
        let objects = actual_payload.objects_for_shard(shard);

        let mut local_pledges = vec![];
        for (object, change, claim) in objects {
            if !claim.is_valid(payload) {
                return Err("Claim is not valid".to_string());
            }
            local_pledges.push(self.shard_db.pledge_object(shard, object, change, payload, leaf_height));
        }
        let leaf_node = self.create_leaf(
            leaf,
            shard,
            payload,
            qc,
            epoch,
            self.identity.clone(),
            NodeHeight(leaf_height.0 + 1),
            payload_height,
            local_pledges,
        );
        self.shard_db.save_node(leaf_node.clone());

        self.tx_broadcast
            .send((HotStuffMessage::generic(leaf_node.clone(), shard), members))
            .await
            .unwrap();
        Ok(leaf_node)
    }

    fn create_leaf(
        &self,
        parent: TreeNodeHash,
        shard: ShardId,
        payload: PayloadId,
        qc: QuorumCertificate,
        epoch: Epoch,
        leader: TAddr,
        height: NodeHeight,
        payload_height: NodeHeight,
        local_pledges: Vec<ObjectPledge>,
    ) -> HotStuffTreeNode<TAddr> {
        HotStuffTreeNode::new(
            parent,
            shard,
            height,
            payload,
            payload_height,
            local_pledges,
            epoch,
            leader,
            qc,
        )
    }

    fn is_leader(&self, payload: PayloadId, shard: ShardId, epoch: Epoch) -> Result<bool, String> {
        Ok(self.leader_strategy.is_leader(
            &self.identity,
            &self.epoch_manager.get_committee(epoch, shard)?,
            payload,
            shard,
            0,
        ))
    }

    fn validate_from_committee(&self, from: &TAddr, epoch: Epoch, shard: ShardId) -> Result<(), String> {
        if self.epoch_manager.get_committee(epoch, shard)?.contains(from) {
            Ok(())
        } else {
            Err("From is not part of this committee".to_string())
        }
    }

    fn validate_qc(&self, qc: &QuorumCertificate) -> Result<(), String> {
        // TODO: get committee at epoch
        // TODO: Validate committee signatures
        Ok(())
    }

    async fn on_next_sync_view(&mut self, payload: TPayload, shard: ShardId) -> Result<(), String> {
        dbg!("new payload received");

        // get state
        let high_qc = self.get_highest_qc(shard);
        // send to leader

        let new_view = HotStuffMessage::new_view(high_qc, shard, Some(payload));

        self.tx_leader.send(new_view).await.map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn update_nodes(&mut self, node: HotStuffTreeNode<TAddr>, shard: ShardId) -> Result<(), String> {
        dbg!("Update nodes");
        if node.justify().local_node_hash() == TreeNodeHash::zero() {
            dbg!("Node is parented to genesis, no need to update");
            return Ok(());
        }
        self.shard_db.update_high_qc(node.justify().clone());
        let b_two = self
            .shard_db
            .node(&node.justify().local_node_hash())
            .ok_or("No node b2")?
            .clone();

        if b_two.justify().local_node_hash() == TreeNodeHash::zero() {
            dbg!("b one is genesis, nothing to do");
            return Ok(());
        }
        dbg!(&b_two);
        let b_one = self
            .shard_db
            .node(&b_two.justify().local_node_hash())
            .ok_or("No node b1")?
            .clone();

        dbg!(&b_one);
        let (b_lock, b_lock_height) = self.shard_db.get_locked_node_hash_and_height(shard);
        if b_one.height().0 > b_lock_height.0 {
            // commit
            dbg!("Commiting height", b_one.height());
            self.shard_db.set_locked(shard, b_one.hash().clone(), b_one.height());
        }
        // if b_one.justify().local_node_hash() == &TreeNodeHash::zero() {
        //     dbg!("b is genesis, nothing to do");
        //     return Ok(());
        // }
        // let b = self
        //     .shard_db
        //     .node(b_one.justify().node_hash())
        //     .ok_or("No node b")?
        //     .clone();
        // dbg!(&b);
        if node.justify().payload_height() == NodeHeight(4) {
            // decide
            dbg!("Deciding height:", node.height());
            self.on_commit(node, shard).await?;
        }
        Ok(())
    }

    #[async_recursion]
    async fn on_commit(&mut self, node: HotStuffTreeNode<TAddr>, shard: ShardId) -> Result<(), String> {
        if self.shard_db.get_last_executed_height(shard) < node.height() {
            if node.parent() != &TreeNodeHash::zero() {
                let parent = self.shard_db.node(node.parent()).ok_or("No parent node")?;
                dbg!("Committing parent");
                self.on_commit(parent.clone(), shard).await?;
            }
            if node.justify().payload_height() == NodeHeight(4) {
                let payload = self
                    .shard_db
                    .get_payload(&node.justify().payload())
                    .ok_or("No payload")?;
                self.execute(payload.clone()).await?;
            }
            self.shard_db.set_last_executed_height(shard, node.height());
        }
        Ok(())
    }

    async fn execute(&mut self, payload: TPayload) -> Result<(), String> {
        self.tx_execute
            .send(payload)
            .await
            .map_err(|e| format!("Could not send execute cmd:{}", e))
    }

    fn validate_proposal(&self, node: &HotStuffTreeNode<TAddr>) -> Result<(), String> {
        if node.payload_height() != NodeHeight(0) &&
            !(node.payload() == node.justify().payload() &&
                node.payload_height() == node.justify().payload_height() + NodeHeight(1))
        {
            Err("Node payload does not match justify payload".to_string())
        } else {
            Ok(())
        }
    }

    async fn on_receive_proposal(&mut self, from: TAddr, node: HotStuffTreeNode<TAddr>) -> Result<(), String> {
        // TODO: validate message from leader
        // TODO: Validate I am processing this shard
        // TODO: Validate the epoch is still valid
        self.validate_proposal(&node)?;

        let shard = node.shard();
        self.shard_db.save_node(node.clone());
        let v_height = self.shard_db.get_last_voted_height(shard);
        // TODO: can also use the QC and committee to justify this....
        let (locked_node, locked_height) = self.shard_db.get_locked_node_hash_and_height(shard);
        if node.height() > v_height &&
            (node.parent() == &locked_node || node.justify().local_node_height() > locked_height)
        {
            dbg!("can save payload vote");
            self.shard_db
                .save_payload_vote(shard, node.payload(), node.payload_height(), node.clone());

            let payload = self
                .shard_db
                .get_payload(&node.payload())
                .ok_or("No payload found".to_string())?;
            let involved_shards = payload.involved_shards();
            let mut votes = vec![];
            dbg!(involved_shards);
            for s in involved_shards {
                if let Some(vote) = self
                    .shard_db
                    .get_payload_vote(node.payload(), node.payload_height(), *s)
                {
                    votes.push((shard, vote.hash().clone()));
                } else {
                    break;
                }
            }
            dbg!(&votes);

            if votes.len() == involved_shards.len() {
                let local_shards = self
                    .epoch_manager
                    .get_shards(node.epoch(), &self.identity, involved_shards)?;
                // it may happen that we are involved in more than one committee, in which case send the votes to each
                // leader.
                for local_shard in local_shards {
                    dbg!("Can vote on the message");
                    let local_node = self
                        .shard_db
                        .get_payload_vote(node.payload(), node.payload_height(), local_shard)
                        .unwrap();

                    self.shard_db.set_last_voted_height(local_shard, local_node.height());

                    let signature = ValidatorSignature::from_bytes(&self.sign(node.hash(), shard));
                    let vote_msg = VoteMessage {
                        local_node_hash: local_node.hash().clone(),
                        shard: local_shard,
                        other_shard_nodes: votes.clone(),
                        signature,
                    };

                    self.tx_vote_message
                        .send((
                            vote_msg,
                            local_node.proposed_by().clone(), // self.get_leader(),
                        ))
                        .await
                        .map_err(|e| e.to_string())?;
                }
            }
        } else {
            dbg!("Invalid proposal");
            dbg!("ignoring");
        }
        self.update_nodes(node.clone(), shard).await?;
        Ok(())
    }

    fn sign(&self, node_hash: &TreeNodeHash, shard: ShardId) -> Vec<u8> {
        // todo!();
        vec![]
    }

    // The leader receives votes from his local shard, and forwards it to all other shards
    async fn on_receive_vote(&mut self, from: TAddr, msg: VoteMessage) -> Result<(), String> {
        // TODO: Only do this if you're the leader
        if self
            .shard_db
            .has_vote_for(&from, msg.local_node_hash.clone(), msg.shard)
        {
            return Ok(());
        }

        let node = self
            .shard_db
            .node(&msg.local_node_hash)
            .ok_or("Could not find node, was it saved previously?".to_string())
            .expect("should have been saved?");

        if node.proposed_by() != &self.identity {
            return Err("I am not the leader for this node".to_string());
        }

        let valid_committee = self.epoch_manager.get_committee(node.epoch(), node.shard())?;

        if !valid_committee.contains(&from) {
            return Err("Not a valid committee member".to_string());
        }

        let total_votes = self
            .shard_db
            .save_vote_for(from, msg.local_node_hash.clone(), msg.shard, msg.signature);
        // Check for consensus
        dbg!(total_votes);
        if total_votes >= valid_committee.consensus_threshold() {
            todo!("Need to check that they all voted for the same other nodes");
            // let signatures = self.shard_db.get_signatures_for(msg.node_hash.clone(), msg.shard);
            //
            // let qc = QuorumCertificate::new(
            //     node.payload(),
            //     node.payload_height(),
            //     node.hash(),
            //     node.height(),
            //     node.shard(),
            //     node.epoch(),
            //     node.involved_shards(),
            //     signatures.iter().map(|(_, sig)| sig.clone()).collect(),
            // );
            // self.shard_db.update_high_qc(qc);
            // // Should be the pace maker actually
            // self.on_beat(msg.shard, None).await?;
        }
        Ok(())
    }

    // fn get_leader(&self, payload: Option<&TPayload>, shard: u32) -> &TAddr {
    //     self.leader_strategy.get_leader(&self.committee, payload, shard)
    // }

    pub async fn run(mut self, mut rx_shutdown: Receiver<()>) -> Result<(), String> {
        loop {
            tokio::select! {
                msg = self.rx_new.recv() => {
                    if let Some((p, shard)) = msg {
                        self.on_next_sync_view(p.clone(), shard).await?;
                        // self.on_beat(0, msg);
                        // TODO: Start timer for receiving proposal
                    } else {
                        dbg!("All senders have dropped");
                    }
                },
                msg = self.rx_hs_message.recv() => {
                    if let Some((from, msg) ) = msg {
                        match msg.message_type() {
                            HotStuffMessageType::NewView => {
                                if let Some(payload) = msg.new_view_payload() {
                                    self.on_receive_new_view(from, msg.shard(), msg.high_qc().unwrap(), payload.clone());
                                    // There should always be a payload, otherwise the leader
                                    // can't be determined
                                    match self.on_beat(msg.shard(), payload.to_id()).await {
                                        Ok(()) => {},
                                        Err(e) => {
                                            dbg!(e);
                                        }
                                    }
                                }
                            },
                            HotStuffMessageType::Generic => {
                                if let Some(node) = msg.node() {
                                    match self.on_receive_proposal(from, node.clone()).await {
                                        Ok(()) => {},
                                        Err(e) => {
                                            dbg!(e);
                                        }
                                    }
                                } else {
                                    dbg!("No node supplied");
                                }
                            }
                            _ => todo!()
                        }
                    }
                },
                msg = self.rx_votes.recv() => {
                    if let Some((from, msg)) = msg {
                        dbg!("Received vote");
                        self.on_receive_vote(from, msg).await?;
                    }
                },
                _ = rx_shutdown.recv() => {
                    dbg!("Exiting");
                    break;
                }
            }
        }
        Ok(())
    }
}

pub struct HsTestHarness<TPayload: Payload + 'static, TAddr: NodeAddressable + 'static> {
    tx_new: Sender<(TPayload, ShardId)>,
    tx_hs_messages: Sender<(TAddr, HotStuffMessage<TPayload, TAddr>)>,
    rx_leader: Receiver<HotStuffMessage<TPayload, TAddr>>,
    tx_shutdown: Sender<()>,
    rx_broadcast: Receiver<(HotStuffMessage<TPayload, TAddr>, Vec<TAddr>)>,
    rx_vote_message: Receiver<(VoteMessage, TAddr)>,
    tx_votes: Sender<(TAddr, VoteMessage)>,
    rx_execute: Receiver<TPayload>,
    hs_waiter: Option<JoinHandle<Result<(), String>>>,
}
impl<TPayload: Payload, TAddr: NodeAddressable> HsTestHarness<TPayload, TAddr> {
    pub fn new<
        TEpochManager: EpochManager<TAddr> + Send + 'static,
        TLeader: LeaderStrategy<TAddr, TPayload> + Send + 'static,
    >(
        identity: TAddr,
        epoch_manager: TEpochManager,
        leader: TLeader,
    ) -> Self {
        let (tx_new, rx_new) = channel(1);
        let (tx_hs_messages, rx_hs_messages) = channel(1);
        let (tx_leader, mut rx_leader) = channel(1);
        let (tx_shutdown, rx_shutdown) = channel(1);
        let (tx_broadcast, rx_broadcast) = channel(1);
        let (tx_vote_message, rx_vote_message) = channel(1);
        let (tx_votes, rx_votes) = channel(1);
        let (tx_execute, rx_execute) = channel(1);

        let hs_waiter = Some(HotStuffWaiter::<_, _, _, _>::spawn(
            identity,
            epoch_manager,
            leader,
            rx_new,
            rx_hs_messages,
            rx_votes,
            tx_leader,
            tx_broadcast,
            tx_vote_message,
            tx_execute,
            rx_shutdown,
        ));
        Self {
            tx_new,
            tx_hs_messages,
            rx_leader,
            tx_shutdown,
            rx_broadcast,
            rx_vote_message,
            tx_votes,
            rx_execute,
            hs_waiter,
        }
    }

    async fn assert_shuts_down_safely(&mut self) {
        // send might fail if it's already shutdown
        let _ = self.tx_shutdown.send(()).await;
        self.hs_waiter.take().unwrap().await.expect("did not end cleanly");
    }

    async fn recv_broadcast(&mut self) -> (HotStuffMessage<TPayload, TAddr>, Vec<TAddr>) {
        if let Some(msg) = timeout(Duration::from_secs(10), self.rx_broadcast.recv())
            .await
            .expect("timed out")
        {
            msg
        } else {
            // Otherwise there are no senders, meaning the main loop has shut down,
            // so try shutdown to get the actual error
            self.assert_shuts_down_safely().await;
            panic!("Shut down safely, but still received none");
        }
    }

    async fn recv_vote_message(&mut self) -> (VoteMessage, TAddr) {
        if let Some(msg) = timeout(Duration::from_secs(10), self.rx_vote_message.recv())
            .await
            .expect("timed out")
        {
            msg
        } else {
            // Otherwise there are no senders, meaning the main loop has shut down,
            // so try shutdown to get the actual error
            self.assert_shuts_down_safely().await;
            panic!("Shut down safely, but still received none");
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_receives_new_payload_starts_new_chain() {
    let node1 = "node1".to_string();
    let epoch_manager = RangeEpochManager::new(ShardId(0)..ShardId(1), vec![node1.clone()]);
    let mut instance = HsTestHarness::new(node1.clone(), epoch_manager, AlwaysFirstLeader {});

    let new_payload = ("Hello world".to_string(), vec![ShardId(0)]);
    instance.tx_new.send((new_payload, ShardId(0))).await.unwrap();
    let leader_message = instance.rx_leader.recv().await.expect("Did not receive leader message");
    dbg!(leader_message);
    instance.assert_shuts_down_safely().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_hs_waiter_leader_proposes() {
    let node1 = "node1".to_string();
    let node2 = "node2".to_string();
    let epoch_manager = RangeEpochManager::new(ShardId(0)..ShardId(1), vec![node1.clone(), node2.clone()]);
    let mut instance = HsTestHarness::new(node1.clone(), epoch_manager, AlwaysFirstLeader {});
    let payload = ("Hello World".to_string(), vec![ShardId(0)]);

    dbg!(payload.to_id());
    // Send a new view message
    let new_view_message = HotStuffMessage::new_view(QuorumCertificate::genesis(), ShardId(0), Some(payload));

    instance
        .tx_hs_messages
        .send((node1.clone(), new_view_message))
        .await
        .unwrap();

    let (_, broadcast_group) = instance.recv_broadcast().await;

    assert_eq!(broadcast_group, vec![node1, node2]);
    instance.assert_shuts_down_safely().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_hs_waiter_replica_sends_vote_for_proposal() {
    let node1 = "node1".to_string();
    let node2 = "node2".to_string();
    let epoch_manager = RangeEpochManager::new(ShardId(0)..ShardId(1), vec![node1.clone(), node2.clone()]);
    let mut instance = HsTestHarness::new(node1.clone(), epoch_manager, AlwaysFirstLeader {});
    let payload = ("Hello World".to_string(), vec![ShardId(0)]);
    let new_view_message = HotStuffMessage::new_view(QuorumCertificate::genesis(), ShardId(0), Some(payload));

    // Node 2 sends new view to node 1
    instance
        .tx_hs_messages
        .send((node2, new_view_message.clone()))
        .await
        .unwrap();

    // Should receive a proposal
    let (proposal_message, broadcast_group) = instance.recv_broadcast().await;

    // Forward the proposal back to itself
    instance
        .tx_hs_messages
        .send((node1.clone(), proposal_message))
        .await
        .expect("Should not error");

    let (vote, from) = instance.recv_vote_message().await;

    dbg!(vote);
    assert_eq!(from, node1);
    // todo!("assert values");
    instance.assert_shuts_down_safely().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_hs_waiter_leader_sends_new_proposal_when_enough_votes_are_received() {
    let node1 = "node1".to_string();
    let node2 = "node2".to_string();
    let epoch_manager = RangeEpochManager::new(ShardId(0)..ShardId(1), vec![node1.clone(), node2.clone()]);
    let mut instance = HsTestHarness::new(node1.clone(), epoch_manager, AlwaysFirstLeader {});
    let payload = ("Hello World".to_string(), vec![ShardId(0)]);

    // Start a new view
    let new_view_message = HotStuffMessage::new_view(QuorumCertificate::genesis(), ShardId(0), Some(payload));
    instance
        .tx_hs_messages
        .send((node2.clone(), new_view_message.clone()))
        .await
        .unwrap();

    // Get the node hash from the proposal
    let (proposal_message, broadcast_group) = instance.recv_broadcast().await;

    // tx_hs_messages
    //     .send((node1.clone(), proposal_message))
    //     .await
    //     .expect("Should not error");

    let vote_hash = proposal_message.node().unwrap().hash().clone();

    // Create some votes
    let vote = VoteMessage {
        local_node_hash: vote_hash.clone(),
        shard: ShardId(0),
        other_shard_nodes: Default::default(),
        signature: ValidatorSignature {
            signer: node1.clone().into_bytes(),
        },
    };
    instance.tx_votes.send((node1, vote.clone())).await.unwrap();

    // Should get no proposal
    assert!(
        timeout(Duration::from_secs(1), instance.rx_broadcast.recv())
            .await
            .is_err(),
        "received a proposal when we weren't expecting it"
    );

    // Send another vote
    let vote = VoteMessage {
        local_node_hash: vote_hash.clone(),
        shard: ShardId(0),
        other_shard_nodes: Default::default(),
        signature: ValidatorSignature {
            signer: node2.clone().into_bytes(),
        },
    };
    instance.tx_votes.send((node2, vote)).await.unwrap();

    // should get a proposal

    let (proposal2, broadcast_group) = instance.recv_broadcast().await;

    let proposed_node = proposal2.node().expect("Should have a node attached");

    assert_eq!(proposed_node.justify().local_node_hash(), vote_hash);

    instance.assert_shuts_down_safely().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_hs_waiter_execute_called_when_consensus_reached() {
    let node1 = "node1".to_string();
    let epoch_manager = RangeEpochManager::new(ShardId(0)..ShardId(1), vec![node1.clone()]);
    let mut instance = HsTestHarness::new(node1.clone(), epoch_manager, AlwaysFirstLeader {});
    let payload = ("Hello World".to_string(), vec![ShardId(0)]);

    let new_view_message = HotStuffMessage::new_view(QuorumCertificate::genesis(), ShardId(0), Some(payload.clone()));
    instance
        .tx_hs_messages
        .send((node1.clone(), new_view_message.clone()))
        .await
        .unwrap();

    // Get the node hash from the proposal
    let (proposal1, broadcast_group) = instance.recv_broadcast().await;

    // loopback the proposal
    instance.tx_hs_messages.send((node1.clone(), proposal1)).await.unwrap();
    let (vote, _) = timeout(Duration::from_secs(10), instance.rx_vote_message.recv())
        .await
        .expect("timedout")
        .expect("should not be none");
    // loopback the vote
    instance.tx_votes.send((node1.clone(), vote.clone())).await.unwrap();
    let (proposal2, broadcast_group) = instance.recv_broadcast().await;

    // loopback the proposal
    instance.tx_hs_messages.send((node1.clone(), proposal2)).await.unwrap();
    let (vote, _) = timeout(Duration::from_secs(10), instance.rx_vote_message.recv())
        .await
        .expect("timedout")
        .expect("should not be none");

    // No execute yet
    assert!(
        timeout(Duration::from_secs(1), instance.rx_execute.recv())
            .await
            .is_err(),
        "received an execute when we weren't expecting it"
    );

    instance.tx_votes.send((node1.clone(), vote.clone())).await.unwrap();

    let (proposal3, broadcast_group) = instance.recv_broadcast().await;

    // loopback the proposal
    instance.tx_hs_messages.send((node1.clone(), proposal3)).await.unwrap();
    let (vote, _) = timeout(Duration::from_secs(10), instance.rx_vote_message.recv())
        .await
        .expect("timedout")
        .expect("should not be none");

    // No execute yet
    assert!(
        timeout(Duration::from_secs(1), instance.rx_execute.recv())
            .await
            .is_err(),
        "received an execute when we weren't expecting it"
    );
    // loopback the vote
    instance.tx_votes.send((node1.clone(), vote.clone())).await.unwrap();

    let (proposal4, broadcast_group) = instance.recv_broadcast().await;

    instance.tx_hs_messages.send((node1.clone(), proposal4)).await.unwrap();
    let (vote, _) = timeout(Duration::from_secs(10), instance.rx_vote_message.recv())
        .await
        .expect("timedout")
        .expect("should not be none");

    // // No execute yet
    // assert!(
    //     timeout(Duration::from_secs(1), rx_execute.recv()).await.is_err(),
    //     "received an execute when we weren't expecting it"
    // );
    //
    // tx_votes.send((node1, vote.clone())).await.unwrap();
    //
    let executed_payload = timeout(Duration::from_secs(10), instance.rx_execute.recv())
        .await
        .expect("timed out")
        .expect("Should not be None");

    assert_eq!(executed_payload, payload);
    instance.assert_shuts_down_safely().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_hs_waiter_multishard_votes() {
    let node1 = "node1".to_string();
    let node2 = "node2".to_string();
    let node3 = "node3".to_string();
    let node4 = "node4".to_string();
    let shard0_committee = vec![node1.clone(), node2.clone()];
    let shard1_committee = vec![node3.clone(), node4.clone()];
    let epoch_manager = RangeEpochManager::new_with_multiple(&vec![
        (ShardId(0)..ShardId(1), shard0_committee),
        (ShardId(1)..ShardId(2), shard1_committee),
    ]);
    let mut node1_instance = HsTestHarness::new(node1.clone(), epoch_manager.clone(), AlwaysFirstLeader {});
    let mut node3_instance = HsTestHarness::new(node3.clone(), epoch_manager, AlwaysFirstLeader {});

    let payload = ("Hello World".to_string(), vec![ShardId(0), ShardId(1)]);

    let new_view_message = HotStuffMessage::new_view(QuorumCertificate::genesis(), ShardId(0), Some(payload.clone()));
    node1_instance
        .tx_hs_messages
        .send((node1.clone(), new_view_message.clone()))
        .await
        .unwrap();

    node3_instance
        .tx_hs_messages
        .send((node3.clone(), new_view_message.clone()))
        .await
        .unwrap();

    // Get the node hash from the proposal
    let (proposal_message, broadcast_group) = node1_instance.recv_broadcast().await;

    let executed_payload = timeout(Duration::from_secs(10), node1_instance.rx_execute.recv())
        .await
        .expect("timed out")
        .expect("Should not be None");

    assert_eq!(executed_payload, payload);
    node1_instance.assert_shuts_down_safely().await;
    node3_instance.assert_shuts_down_safely().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_hs_waiter_leader_starts_view_with_n_minus_f_new_view() {
    // TODO: I don't know if this is a requirement, the prepare step might actually be fine
    // let (tx_new, rx_new) = channel(1);
    // let (tx_hs_messages, rx_hs_messages) = channel(1);
    // let (tx_leader, mut rx_leader) = channel(1);
    // let (tx_broadcast, mut rx_broadcast) = channel(1);
    // let (tx_shutdown, rx_shutdown) = channel(1);
    // let node1 = "node1".to_string();
    // let node2 = "node2".to_string();
    // let node3 = "node3".to_string();
    // let node4 = "node4".to_string();
    // let committee = Committee::new(vec![node1.clone(), node2.clone(), node3.clone(), node4.clone()]);
    //
    // let instance = HotStuffWaiter::<String, String>::spawn(
    //     node3.clone(),
    //     committee,
    //     rx_new,
    //     rx_hs_messages,
    //     tx_leader,
    //     tx_broadcast,
    //     rx_shutdown,
    // );
    // let payload = "Hello World".to_string();
    //
    // Send a new view message
    // let new_view_message = HotStuffMessage::new_view(QuorumCertificate::genesis(0), 0, Some(payload));
    //
    // tx_hs_messages.send((node1, new_view_message.clone())).await.unwrap();
    // tx_hs_messages.send((node2, new_view_message.clone())).await.unwrap();

    // should receive a broadcast proposal
    // let proposal_message = rx_broadcast.try_recv().expect("Did not receive proposal");
    // assert!(
    //     timeout(Duration::from_secs(1), rx_broadcast.recv()).await.is_err(),
    //     "Leader should not have proposed until it's received 3 messages"
    // );

    // Technically the leader will send this to themselves
    // tx_hs_messages.send((node3, new_view_message)).await.unwrap();

    // Now we should receive the proposal
    // let proposal_message = timeout(Duration::from_secs(10), rx_broadcast.recv())
    //     .await
    //     .expect("timed out");

    // tx_shutdown.send(()).await.unwrap();
    // instance.await.expect("did not end cleanly");
    todo!()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_hs_waiter_non_committee_member_does_not_start_new_view() {
    todo!()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_hs_waiter_validate_qc_for_incorrect_committee_fails() {
    todo!()
}

// async fn recv_timeout<'a, T>(channel: &'a mut Receiver<T>, duration: Duration<>) -> Result<Option<T>, String> {
//     let timeout = tokio::time::timeout(duration);
//     tokio::select! {
//         msg = channel.recv() => {
//             Ok(msg)
//         },
//         _ = timeout => {
//             Err("Timed out".to_string())
//         }
//     }
// }

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_hs_waiter_cannot_spend_until_it_is_proven_committed() {
    // You must provide a valid 4 chain proof in order to spend or exist an output
    todo!()
}
