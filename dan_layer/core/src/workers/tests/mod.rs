use std::{
    collections::HashMap,
    ops::{Index, Range},
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
        HotStuffMessage,
        HotStuffMessageType,
        HotStuffMessageType::Commit,
        HotStuffTreeNode,
        Payload,
        QuorumCertificate,
        TariDanPayload,
        TreeNodeHash,
        ValidatorSignature,
        ViewId,
    },
    services::infrastructure_services::NodeAddressable,
};

pub struct ShardDb<TAddr: NodeAddressable, TPayload: Payload> {
    // replica data
    shard_high_qcs: HashMap<u32, QuorumCertificate>,
    // pace maker data
    shard_leaf_nodes: HashMap<u32, (TreeNodeHash, u32)>,
    last_voted_heights: HashMap<u32, u32>,
    lock_node_and_heights: HashMap<u32, (TreeNodeHash, u32)>,
    votes: HashMap<(TreeNodeHash, u32), Vec<(TAddr, ValidatorSignature)>>,
    nodes: HashMap<TreeNodeHash, HotStuffTreeNode<TPayload, TAddr>>,
    last_executed_height: HashMap<u32, u32>,
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
        }
    }

    pub fn get_high_qc_for(&self, shard: u32) -> QuorumCertificate {
        if let Some(qc) = self.shard_high_qcs.get(&shard) {
            qc.clone()
        } else {
            QuorumCertificate::genesis(shard)
        }
    }

    pub fn update_high_qc(&mut self, qc: QuorumCertificate) {
        let entry = self.shard_high_qcs.entry(qc.shard()).or_insert(qc.clone());
        if qc.node_height() > entry.node_height() {
            *entry = qc.clone();
            self.shard_leaf_nodes
                .entry(qc.shard())
                .and_modify(|e| *e = (qc.node_hash().clone(), qc.node_height()))
                .or_insert((qc.node_hash().clone(), qc.node_height()));
        }
    }

    pub fn get_leaf_node(&self, shard: u32) -> (TreeNodeHash, u32) {
        if let Some(leaf) = self.shard_leaf_nodes.get(&shard) {
            leaf.clone()
        } else {
            (TreeNodeHash::zero(), 0)
        }
    }

    pub fn update_leaf_node(&mut self, shard: u32, node: TreeNodeHash, height: u32) -> Result<(), String> {
        let leaf = self.shard_leaf_nodes.entry(shard).or_insert((node, height));
        *leaf = (node, height);
        Ok(())
    }

    pub fn get_last_voted_height(&self, shard: u32) -> u32 {
        *self.last_voted_heights.get(&shard).unwrap_or(&0)
    }

    pub fn set_last_voted_height(&mut self, shard: u32, height: u32) {
        let entry = self.last_voted_heights.entry(shard).or_insert(height);
        *entry = height;
    }

    pub fn get_locked_node_hash_and_height(&self, shard: u32) -> (TreeNodeHash, u32) {
        self.lock_node_and_heights
            .get(&shard)
            .unwrap_or(&(TreeNodeHash::zero(), 0))
            .clone()
    }

    pub fn set_locked(&mut self, shard: u32, node_hash: TreeNodeHash, node_height: u32) {
        self.lock_node_and_heights
            .entry(shard)
            .and_modify(|e| *e = (node_hash, node_height));
    }

    pub fn has_vote_for(&self, from: &TAddr, node_hash: TreeNodeHash, shard: u32) -> bool {
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
        shard: u32,
        signature: ValidatorSignature,
    ) -> usize {
        let entry = self.votes.entry((node_hash, shard)).or_insert(vec![]);
        entry.push((from, signature));
        entry.len()
    }

    pub fn get_signatures_for(&self, node_hash: TreeNodeHash, shard: u32) -> Vec<(TAddr, ValidatorSignature)> {
        if let Some(sigs) = self.votes.get(&(node_hash, shard)) {
            sigs.clone()
        } else {
            vec![]
        }
    }

    pub fn save_node(&mut self, node: HotStuffTreeNode<TPayload, TAddr>) {
        self.nodes.entry(node.hash().clone()).or_insert(node.clone());
    }

    pub fn node(&self, node_hash: &TreeNodeHash) -> Option<&HotStuffTreeNode<TPayload, TAddr>> {
        self.nodes.get(node_hash)
    }

    pub fn set_last_executed_height(&mut self, shard: u32, height: u32) {
        self.last_executed_height.entry(shard).and_modify(|e| *e = height);
    }

    pub fn get_last_executed_height(&self, shard: u32) -> u32 {
        *self.last_executed_height.get(&shard).unwrap_or(&0)
    }
}

#[derive(Debug, Clone)]
pub struct VoteMessage {
    pub main_node_hash: TreeNodeHash,
    pub shard: u32,
    pub other_shard_nodes: HashMap<u32, TreeNodeHash>,
    pub signature: ValidatorSignature,
}

pub trait LeaderStrategy<TAddr: NodeAddressable, TPayload> {
    fn calculate_leader(&self, committee: &Committee<TAddr>, payload: Option<&TPayload>, shard: u32) -> usize;
    fn is_leader(&self, node: &TAddr, committee: &Committee<TAddr>, payload: Option<&TPayload>, shard: u32) -> bool {
        let position = self.calculate_leader(committee, payload, shard);
        if let Some(index) = committee.members.iter().position(|m| m == node) {
            position == index
        } else {
            false
        }
    }

    fn get_leader<'a, 'b>(
        &'a self,
        committee: &'b Committee<TAddr>,
        payload: Option<&TPayload>,
        shard: u32,
    ) -> &'b TAddr {
        let index = self.calculate_leader(committee, payload, shard);
        committee.members.get(index).unwrap()
    }
}

pub struct AlwaysFirstLeader {}

impl<TAddr: NodeAddressable, TPayload> LeaderStrategy<TAddr, TPayload> for AlwaysFirstLeader {
    fn calculate_leader(&self, committee: &Committee<TAddr>, payload: Option<&TPayload>, shard: u32) -> usize {
        0
    }
}

pub trait EpochManager<TAddr: NodeAddressable> {
    fn current_epoch(&self) -> u32;
    fn is_epoch_valid(&self, epoch: u32) -> bool;
    fn get_committees(&self, epoch: u32, shards: &[u32]) -> Result<Vec<(u32, Option<Committee<TAddr>>)>, String>;
    fn get_committee(&self, epoch: u32, shard: u32) -> Result<Committee<TAddr>, String>;
}

pub struct RangeEpochManager<TAddr: NodeAddressable> {
    current_epoch: u32,
    epochs: HashMap<u32, Vec<(Range<u32>, Committee<TAddr>)>>,
}

impl<TAddr: NodeAddressable> EpochManager<TAddr> for RangeEpochManager<TAddr> {
    fn current_epoch(&self) -> u32 {
        self.current_epoch
    }

    fn is_epoch_valid(&self, epoch: u32) -> bool {
        self.current_epoch == epoch
    }

    fn get_committees(&self, epoch: u32, shards: &[u32]) -> Result<Vec<(u32, Option<Committee<TAddr>>)>, String> {
        let epoch = self.epochs.get(&epoch).ok_or("No value for that epoch".to_string())?;
        let mut result = vec![];
        for shard in shards {
            let mut found_committee = None;
            for (range, committee) in epoch {
                if range.includes(shard) {
                    found_committee = Some(committee.clone());
                    break;
                }
                result.push((shard, found_committee));
            }
        }

        Ok(result)
    }

    fn get_committee(&self, epoch: u32, shard: u32) -> Result<Committee<TAddr>, String> {
        let epoch = self.epochs.get(&epoch).ok_or("No value for that epoch".to_string())?;
        for (range, committee) in epoch {
            if range.includes(shard) {
                return Ok(committee.clone());
            }
        }
        Err("Could not find a committee for that shard".to_string())
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
    rx_new: Receiver<TPayload>,
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
        TEpochManager: EpochManager<TAddr> + 'static,
    > HotStuffWaiter<TPayload, TAddr, TLeaderStrategy, TEpochManager>
{
    pub fn spawn(
        identity: TAddr,
        epoch_manager: TEpochManager,
        leader_strategy: TLeaderStrategy,
        rx_new: Receiver<TPayload>,
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
        rx_new: Receiver<TPayload>,
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

    fn get_highest_qc(&self, shard: u32) -> QuorumCertificate {
        self.shard_db.get_high_qc_for(shard)
    }

    // pacemaker
    fn on_receive_new_view(&mut self, from: TAddr, qc: QuorumCertificate) -> Result<(), String> {
        // TODO: Validate who message is from
        self.validate_from_committee(&from);
        self.validate_qc(&qc);
        dbg!("update qc");
        self.shard_db.update_high_qc(qc);
        Ok(())
    }

    // pacemaker
    async fn on_beat(&mut self, shard: u32, payload: Option<TPayload>) -> Result<(), String> {
        // TODO: the leader is only known after the leaf is determines

        if self.is_leader(payload.as_ref(), shard) {
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
        leaf_height: u32,
        shard: u32,
        payload: Option<TPayload>,
    ) -> Result<HotStuffTreeNode<TPayload, TAddr>, String> {
        dbg!("on propose");
        let qc = self.shard_db.get_high_qc_for(shard);
        // TODO: Should the committee members carry on being included until 4 deep? so that they are committed?
        let members;
        let epoch;
        let involved_shards;

        if let Some(payload) = &payload {
            epoch = self.epoch_manager.current_epoch();
            involved_shards = payload.involved_shards().to_vec();
            members = self
                .epoch_manager
                .get_committees(epoch, payload.involved_shards())?
                .into_iter()
                .map(|(shard, committee)| committee.map(|c| c.members).unwrap_or_default())
                .flatten()
                .collect();
        } else {
            epoch = qc.epoch();
            involved_shards = qc.involved_shards().to_vec();
            // Continue the epoch and committee from the QC.
            members = self
                .epoch_manager
                .get_committees(qc.epoch(), qc.involved_shards())?
                .into_iter()
                .map(|(shard, committee)| committee.map(|c| c.members).unwrap_or_default())
                .flatten()
                .collect();
        }

        let leaf_node = self.create_leaf(
            leaf,
            payload,
            qc,
            epoch,
            self.identity.clone(),
            involved_shards,
            leaf_height + 1,
        );
        self.shard_db.save_node(leaf_node.clone());
        dbg!(&leaf_node);

        self.tx_broadcast
            .send((HotStuffMessage::generic(leaf_node.clone(), shard), members))
            .await
            .unwrap();
        Ok(leaf_node)
    }

    fn create_leaf(
        &self,
        parent: TreeNodeHash,
        payload: Option<TPayload>,
        qc: QuorumCertificate,
        epoch: u32,
        leader: TAddr,
        involved_shards: Vec<u32>,
        height: u32,
    ) -> HotStuffTreeNode<TPayload, TAddr> {
        HotStuffTreeNode::new(parent, payload, height, qc.shard(), leader, involved_shards, epoch, qc)
    }

    fn is_leader(&self, payload: Option<&TPayload>, shard: u32) -> bool {
        self.leader_strategy
            .is_leader(&self.identity, &self.committee, payload, shard)
    }

    fn validate_from_committee(&self, from: &TAddr) -> Result<(), String> {
        // Validate that from is in the correct committee
        if !self.committee.contains(from) {
            Err("Not from committee member".to_string())
        } else {
            Ok(())
        }
    }

    fn validate_qc(&self, qc: &QuorumCertificate) -> Result<(), String> {
        // TODO: get committee at epoch
        // TODO: Validate committee signatures
        Ok(())
    }

    async fn on_next_sync_view(&mut self, payload: TPayload) -> Result<(), String> {
        dbg!("new payload received");

        // get state
        let high_qc = self.get_highest_qc(0);
        // send to leader

        let new_view = HotStuffMessage::new_view(high_qc, 1, Some(payload));

        self.tx_leader.send(new_view).await.map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn update_nodes(&mut self, node: HotStuffTreeNode<TPayload, TAddr>, shard: u32) -> Result<(), String> {
        dbg!("Update nodes");
        if node.justify().node_hash() == &TreeNodeHash::zero() {
            dbg!("Node is parented to genesis, no need to update");
            return Ok(());
        }
        self.shard_db.update_high_qc(node.justify().clone());
        let b_two = self
            .shard_db
            .node(node.justify().node_hash())
            .ok_or("No node b2")?
            .clone();

        if b_two.justify().node_hash() == &TreeNodeHash::zero() {
            dbg!("b one is genesis, nothing to do");
            return Ok(());
        }
        dbg!(&b_two);
        let b_one = self
            .shard_db
            .node(b_two.justify().node_hash())
            .ok_or("No node b1")?
            .clone();

        dbg!(&b_one);
        let (b_lock, b_lock_height) = self.shard_db.get_locked_node_hash_and_height(shard);
        if b_one.height() > b_lock_height {
            // commit
            dbg!("Commiting height", b_one.height());
            self.shard_db.set_locked(shard, b_one.hash().clone(), b_one.height());
        }
        if b_one.justify().node_hash() == &TreeNodeHash::zero() {
            dbg!("b is genesis, nothing to do");
            return Ok(());
        }
        let b = self
            .shard_db
            .node(b_one.justify().node_hash())
            .ok_or("No node b")?
            .clone();
        dbg!(&b);
        if b_two.parent() == b_one.hash() && b_one.parent() == b.hash() {
            // decide
            dbg!("Deciding height:", b.height());
            self.on_commit(b.clone(), shard).await?;
            self.shard_db.set_last_executed_height(shard, b.height());
        }
        Ok(())
    }

    #[async_recursion]
    async fn on_commit(&mut self, node: HotStuffTreeNode<TPayload, TAddr>, shard: u32) -> Result<(), String> {
        if self.shard_db.get_last_executed_height(shard) < node.height() {
            if node.parent() != &TreeNodeHash::zero() {
                let parent = self.shard_db.node(node.parent()).ok_or("No parent node")?;
                dbg!("Committing parent");
                self.on_commit(parent.clone(), shard).await?;
            }
            if let Some(payload) = node.payload() {
                self.execute(payload.clone()).await?;
            }
        }
        Ok(())
    }

    async fn execute(&mut self, payload: TPayload) -> Result<(), String> {
        self.tx_execute
            .send(payload)
            .await
            .map_err(|e| format!("Could not send execute cmd:{}", e))
    }

    async fn on_receive_proposal(
        &mut self,
        from: TAddr,
        message: HotStuffMessage<TPayload, TAddr>,
    ) -> Result<(), String> {
        if let Some(node) = message.node() {
            // TODO: validate message from leader
            let shard = message.shard();
            self.shard_db.save_node(node.clone());
            let v_height = self.shard_db.get_last_voted_height(shard);
            let (locked_node, locked_height) = self.shard_db.get_locked_node_hash_and_height(shard);
            // TODO: Change parent check to allow a chain?
            if node.height() > v_height &&
                (node.parent() == &locked_node || node.justify().node_height() > locked_height)
            {
                dbg!("Can vote on the message");
                self.shard_db.set_last_voted_height(shard, node.height());
                let signature = ValidatorSignature::from_bytes(&self.sign(node.hash(), shard));
                self.tx_vote_message
                    .send((
                        VoteMessage {
                            main_node_hash: node.hash().clone(),
                            shard,
                            other_shard_nodes: Default::default(),
                            signature,
                        },
                        // TODO: validate the leader instead of sending to from
                        from, // self.get_leader(),
                    ))
                    .await
                    .map_err(|e| e.to_string())?;
            } else {
                dbg!("Invalid proposal");
                dbg!("ignoring");
            }
            self.update_nodes(node.clone(), shard).await?;
        } else {
            dbg!("No node attached");
        }
        // self.update(message);
        Ok(())
    }

    fn sign(&self, node_hash: &TreeNodeHash, shard: u32) -> Vec<u8> {
        // todo!();
        vec![]
    }

    // The leader receives votes from his local shard, and forwards it to all other shards
    async fn on_receive_vote(&mut self, from: TAddr, msg: VoteMessage) -> Result<(), String> {
        // TODO: Only do this if you're the leader
        if self.shard_db.has_vote_for(&from, msg.node_hash.clone(), msg.shard) {
            return Ok(());
        }

        let node = self
            .shard_db
            .node(&msg.main_node_hash)
            .ok_or("Could not find node, was it saved previously?".to_string())
            .expect("should have been saved?");

        if node.leader() != self.identity {
            return Err("I am not the leader for this node".to_string());
        }

        let valid_committee = self.epoch_manager.get_committee(node.epoch(), node.shard())?;

        if !valid_committee.contains(&from) {
            return Err("Not a valid committee member".to_string());
        }

        let total_votes = self
            .shard_db
            .save_vote_for(from, msg.node_hash.clone(), msg.shard, msg.signature);
        // Check for consensus
        dbg!(total_votes);
        if total_votes >= valid_committee.consensus_threshold() {
            let signatures = self.shard_db.get_signatures_for(msg.node_hash.clone(), msg.shard);

            let qc = QuorumCertificate::new(
                HotStuffMessageType::Generic,
                node.height(),
                msg.node_hash,
                msg.shard,
                node.epoch(),
                node.involved_shards(),
                signatures.iter().map(|(_, sig)| sig.clone()).collect(),
            );
            self.shard_db.update_high_qc(qc);
            // Should be the pace maker actually
            self.on_beat(msg.shard, None).await?;
        }
        Ok(())
    }

    fn get_leader(&self, payload: Option<&TPayload>, shard: u32) -> &TAddr {
        self.leader_strategy.get_leader(&self.committee, payload, shard)
    }

    pub async fn run(mut self, mut rx_shutdown: Receiver<()>) -> Result<(), String> {
        loop {
            tokio::select! {
                msg = self.rx_new.recv() => {
                    if let Some(p) = msg {
                        self.on_next_sync_view(p.clone()).await?;
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
                                self.on_receive_new_view(from, msg.high_qc().unwrap());
                                if let Some(payload) = msg.new_view_payload() {
                                    // There should always be a payload, otherwise the leader
                                    // can't be determined
                                    self.on_beat(0, Some(payload.clone())).await;
                                }
                            },
                            HotStuffMessageType::Generic => {
                                self.on_receive_proposal(from, msg).await?;
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_receives_new_payload_starts_new_chain() {
    let (tx_new, rx_new) = channel(1);
    let (tx_hs_messages, rx_hs_messages) = channel(1);
    let (tx_leader, mut rx_leader) = channel(1);
    let (tx_shutdown, rx_shutdown) = channel(1);
    let (tx_broadcast, rx_broadcast) = channel(1);
    let (tx_vote_message, rx_vote_message) = channel(1);
    let (tx_votes, rx_votes) = channel(1);
    let (tx_execute, rx_execute) = channel(1);
    let instance = HotStuffWaiter::<String, String, _>::spawn(
        "leader".to_string(),
        Committee::empty(),
        AlwaysFirstLeader {},
        rx_new,
        rx_hs_messages,
        rx_votes,
        tx_leader,
        tx_broadcast,
        tx_vote_message,
        tx_execute,
        rx_shutdown,
    );

    let new_payload = "Hello world".to_string();
    tx_new.send(new_payload).await.unwrap();
    let leader_message = rx_leader.recv().await.expect("Did not receive leader message");
    dbg!(leader_message);
    tx_shutdown.send(()).await.unwrap();
    //     let leader_message = rx_leader.recv().await;
    //     dbg!(leader_message);
    instance.await.expect("did not end cleanly");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_hs_waiter_leader_proposes() {
    let (tx_new, rx_new) = channel(1);
    let (tx_hs_messages, rx_hs_messages) = channel(1);
    let (tx_leader, mut rx_leader) = channel(1);
    let (tx_broadcast, mut rx_broadcast) = channel(1);
    let (tx_votes, rx_votes) = channel(1);
    let (tx_vote_message, rx_vote_message) = channel(1);
    let (tx_execute, rx_execute) = channel(1);
    let (tx_shutdown, rx_shutdown) = channel(1);
    let node1 = "node1".to_string();
    let node2 = "node2".to_string();
    let committee = Committee::new(vec![node1.clone(), node2.clone()]);

    let instance = HotStuffWaiter::<String, String, _>::spawn(
        node1.clone(),
        committee,
        AlwaysFirstLeader {},
        rx_new,
        rx_hs_messages,
        rx_votes,
        tx_leader,
        tx_broadcast,
        tx_vote_message,
        tx_execute,
        rx_shutdown,
    );
    let payload = "Hello World".to_string();

    // Send a new view message
    let new_view_message = HotStuffMessage::new_view(QuorumCertificate::genesis(0), 0, Some(payload));

    tx_hs_messages.send((node1.clone(), new_view_message)).await.unwrap();

    // should receive a broadcast proposal
    // let proposal_message = rx_broadcast.try_recv().expect("Did not receive proposal");
    let (proposal_message, broadcast_group) = timeout(Duration::from_secs(10), rx_broadcast.recv())
        .await
        .expect("timed out")
        .expect("Should not be none");

    assert_eq!(broadcast_group, vec![node1, node2]);
    tx_shutdown.send(()).await.unwrap();
    instance.await.expect("did not end cleanly");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_hs_waiter_replica_sends_vote_for_proposal() {
    let (tx_new, rx_new) = channel(1);
    let (tx_hs_messages, rx_hs_messages) = channel(1);
    let (tx_leader, mut rx_leader) = channel(1);
    let (tx_broadcast, mut rx_broadcast) = channel(1);
    let (tx_votes, rx_votes) = channel(1);
    let (tx_vote_message, mut rx_vote_message) = channel(1);
    let (tx_execute, rx_execute) = channel(1);
    let (tx_shutdown, rx_shutdown) = channel(1);
    let node1 = "node1".to_string();
    let node2 = "node2".to_string();
    let committee = Committee::new(vec![node1.clone(), node2.clone()]);

    let instance = HotStuffWaiter::<String, String, _>::spawn(
        node1.clone(),
        committee,
        AlwaysFirstLeader {},
        rx_new,
        rx_hs_messages,
        rx_votes,
        tx_leader,
        tx_broadcast,
        tx_vote_message,
        tx_execute,
        rx_shutdown,
    );
    let payload = "Hello World".to_string();
    let new_view_message = HotStuffMessage::new_view(QuorumCertificate::genesis(0), 0, Some(payload));

    // Node 2 sends new view to node 1
    tx_hs_messages.send((node2, new_view_message.clone())).await.unwrap();

    // Should receive a proposal
    let (proposal_message, broadcast_group) = timeout(Duration::from_secs(10), rx_broadcast.recv())
        .await
        .expect("timed out")
        .expect("Should not be None");

    // Forward the proposal back to itself
    tx_hs_messages
        .send((node1, proposal_message))
        .await
        .expect("Should not error");

    let vote = timeout(Duration::from_secs(10), rx_vote_message.recv())
        .await
        .expect("timed out")
        .expect("should not be none");

    tx_shutdown.send(()).await.unwrap();
    instance.await.expect("did not end cleanly");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_hs_waiter_leader_sends_new_proposal_when_enough_votes_are_received() {
    let (tx_new, rx_new) = channel(1);
    let (tx_hs_messages, rx_hs_messages) = channel(1);
    let (tx_leader, mut rx_leader) = channel(1);
    let (tx_broadcast, mut rx_broadcast) = channel(1);
    let (tx_votes, rx_votes) = channel(1);
    let (tx_vote_message, mut rx_vote_message) = channel(1);
    let (tx_execute, rx_execute) = channel(1);
    let (tx_shutdown, rx_shutdown) = channel(1);
    let node1 = "node1".to_string();
    let node2 = "node2".to_string();
    let committee = Committee::new(vec![node1.clone(), node2.clone()]);

    let instance = HotStuffWaiter::<String, String, _>::spawn(
        node1.clone(),
        committee,
        AlwaysFirstLeader {},
        rx_new,
        rx_hs_messages,
        rx_votes,
        tx_leader,
        tx_broadcast,
        tx_vote_message,
        tx_execute,
        rx_shutdown,
    );
    let payload = "Hello World".to_string();

    // Start a new view
    let new_view_message = HotStuffMessage::new_view(QuorumCertificate::genesis(0), 0, Some(payload));
    tx_hs_messages
        .send((node2.clone(), new_view_message.clone()))
        .await
        .unwrap();

    // Get the node hash from the proposal
    let (proposal_message, broadcast_group) = timeout(Duration::from_secs(10), rx_broadcast.recv())
        .await
        .expect("timed out")
        .expect("Should not be None");

    // tx_hs_messages
    //     .send((node1.clone(), proposal_message))
    //     .await
    //     .expect("Should not error");

    let vote_hash = proposal_message.node().unwrap().hash().clone();

    // Create some votes
    let vote = VoteMessage {
        node_hash: vote_hash.clone(),
        shard: 0,
        signature: ValidatorSignature {
            signer: node1.clone().into_bytes(),
        },
    };
    tx_votes.send((node1, vote.clone())).await.unwrap();

    // Should get no proposal
    assert!(
        timeout(Duration::from_secs(1), rx_broadcast.recv()).await.is_err(),
        "received a proposal when we weren't expecting it"
    );

    // Send another vote
    let vote = VoteMessage {
        node_hash: vote_hash.clone(),
        shard: 0,
        signature: ValidatorSignature {
            signer: node2.clone().into_bytes(),
        },
    };
    tx_votes.send((node2, vote)).await.unwrap();

    // should get a proposal

    let (proposal2, broadcast_group) = timeout(Duration::from_secs(10), rx_broadcast.recv())
        .await
        .expect("timed out")
        .expect("Should not be None");

    let proposed_node = proposal2.node().expect("Should have a node attached");

    assert_eq!(proposed_node.justify().node_hash(), &vote_hash);

    tx_shutdown.send(()).await.unwrap();
    instance.await.expect("did not end cleanly");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_hs_waiter_execute_called_when_consensus_reached() {
    let (tx_new, rx_new) = channel(1);
    let (tx_hs_messages, rx_hs_messages) = channel(1);
    let (tx_leader, mut rx_leader) = channel(1);
    let (tx_broadcast, mut rx_broadcast) = channel(1);
    let (tx_votes, rx_votes) = channel(1);
    let (tx_vote_message, mut rx_vote_message) = channel(1);
    let (tx_execute, mut rx_execute) = channel(1);
    let (tx_shutdown, rx_shutdown) = channel(1);
    let node1 = "node1".to_string();
    let committee = Committee::new(vec![node1.clone()]);

    let instance = HotStuffWaiter::<String, String, _>::spawn(
        node1.clone(),
        committee,
        AlwaysFirstLeader {},
        rx_new,
        rx_hs_messages,
        rx_votes,
        tx_leader,
        tx_broadcast,
        tx_vote_message,
        tx_execute,
        rx_shutdown,
    );
    let payload = "Hello World".to_string();

    let new_view_message = HotStuffMessage::new_view(QuorumCertificate::genesis(0), 0, Some(payload.clone()));
    tx_hs_messages
        .send((node1.clone(), new_view_message.clone()))
        .await
        .unwrap();

    // Get the node hash from the proposal
    let (proposal1, broadcast_group) = timeout(Duration::from_secs(10), rx_broadcast.recv())
        .await
        .expect("timed out")
        .expect("Should not be None");

    // loopback the proposal
    tx_hs_messages.send((node1.clone(), proposal1)).await.unwrap();
    let (vote, _) = timeout(Duration::from_secs(10), rx_vote_message.recv())
        .await
        .expect("timedout")
        .expect("should not be none");
    // loopback the vote
    tx_votes.send((node1.clone(), vote.clone())).await.unwrap();
    let (proposal2, broadcast_group) = timeout(Duration::from_secs(10), rx_broadcast.recv())
        .await
        .expect("timed out")
        .expect("Should not be None");

    // loopback the proposal
    tx_hs_messages.send((node1.clone(), proposal2)).await.unwrap();
    let (vote, _) = timeout(Duration::from_secs(10), rx_vote_message.recv())
        .await
        .expect("timedout")
        .expect("should not be none");

    // No execute yet
    assert!(
        timeout(Duration::from_secs(1), rx_execute.recv()).await.is_err(),
        "received an execute when we weren't expecting it"
    );

    tx_votes.send((node1.clone(), vote.clone())).await.unwrap();

    let (proposal3, broadcast_group) = timeout(Duration::from_secs(10), rx_broadcast.recv())
        .await
        .expect("timed out")
        .expect("Should not be None");

    // loopback the proposal
    tx_hs_messages.send((node1.clone(), proposal3)).await.unwrap();
    let (vote, _) = timeout(Duration::from_secs(10), rx_vote_message.recv())
        .await
        .expect("timedout")
        .expect("should not be none");

    // No execute yet
    assert!(
        timeout(Duration::from_secs(1), rx_execute.recv()).await.is_err(),
        "received an execute when we weren't expecting it"
    );
    // loopback the vote
    tx_votes.send((node1.clone(), vote.clone())).await.unwrap();

    let (proposal4, broadcast_group) = timeout(Duration::from_secs(10), rx_broadcast.recv())
        .await
        .expect("timed out")
        .expect("Should not be None");

    tx_hs_messages.send((node1.clone(), proposal4)).await.unwrap();
    let (vote, _) = timeout(Duration::from_secs(10), rx_vote_message.recv())
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
    let executed_payload = timeout(Duration::from_secs(10), rx_execute.recv())
        .await
        .expect("timed out")
        .expect("Should not be None");

    assert_eq!(executed_payload, payload);
    tx_shutdown.send(()).await.unwrap();
    instance.await.expect("did not end cleanly");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_hs_waiter_multishard_votes() {
    let (tx_new, rx_new) = channel(1);
    let (tx_hs_messages, rx_hs_messages) = channel(1);
    let (tx_leader, mut rx_leader) = channel(1);
    let (tx_broadcast, mut rx_broadcast) = channel(1);
    let (tx_votes, rx_votes) = channel(1);
    let (tx_vote_message, mut rx_vote_message) = channel(1);
    let (tx_execute, mut rx_execute) = channel(1);
    let (tx_shutdown, rx_shutdown) = channel(1);
    let node1 = "node1".to_string();
    let node2 = "node2".to_string();
    let node3 = "node3".to_string();
    let node4 = "node4".to_string();
    let shard0_committee = Committee::new(vec![node1.clone(), node2.clone()]);
    let shard1_committee = Committee::new(vec![node3.clone(), node4.clone()]);
    let epoch_manager = RangeEpochManager::new(0, vec![(0..1, shard0_committee), (1..2, shard1_committee)]);

    let instance = HotStuffWaiter::<String, String, _>::spawn(
        node1.clone(),
        shard0_committee,
        AlwaysFirstLeader {},
        rx_new,
        rx_hs_messages,
        rx_votes,
        tx_leader,
        tx_broadcast,
        tx_vote_message,
        tx_execute,
        rx_shutdown,
    );
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
