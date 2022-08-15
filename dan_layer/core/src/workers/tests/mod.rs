use std::{collections::HashMap, time::Duration};

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
    nodes: HashMap<TreeNodeHash, HotStuffTreeNode<TPayload>>,
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

    pub fn save_node(&mut self, node: HotStuffTreeNode<TPayload>) {
        self.nodes.entry(node.hash().clone()).or_insert(node.clone());
    }

    pub fn node(&self, node_hash: &TreeNodeHash) -> Option<&HotStuffTreeNode<TPayload>> {
        self.nodes.get(node_hash)
    }
}

#[derive(Debug, Clone)]
pub struct VoteMessage {
    pub node_hash: TreeNodeHash,
    pub shard: u32,
    pub signature: ValidatorSignature,
}

pub struct HotStuffWaiter<TPayload: Payload, TAddr: NodeAddressable> {
    identity: TAddr,
    rx_new: Receiver<TPayload>,
    rx_hs_message: Receiver<(TAddr, HotStuffMessage<TPayload>)>,
    rx_votes: Receiver<(TAddr, VoteMessage)>,
    tx_leader: Sender<HotStuffMessage<TPayload>>,
    tx_broadcast: Sender<(HotStuffMessage<TPayload>, Vec<TAddr>)>,
    tx_vote_message: Sender<(VoteMessage, TAddr)>,
    committee: Committee<TAddr>,
    shard_db: ShardDb<TAddr, TPayload>,
    current_payload: Option<TPayload>,
}

impl<TPayload: Payload + 'static, TAddr: NodeAddressable + 'static> HotStuffWaiter<TPayload, TAddr> {
    pub fn spawn(
        identity: TAddr,
        initial_committee: Committee<TAddr>,
        rx_new: Receiver<TPayload>,
        rx_hs_message: Receiver<(TAddr, HotStuffMessage<TPayload>)>,
        rx_votes: Receiver<(TAddr, VoteMessage)>,
        tx_leader: Sender<HotStuffMessage<TPayload>>,
        tx_broadcast: Sender<(HotStuffMessage<TPayload>, Vec<TAddr>)>,
        tx_vote_message: Sender<(VoteMessage, TAddr)>,
        rx_shutdown: Receiver<()>,
    ) -> JoinHandle<Result<(), String>> {
        tokio::spawn(async move {
            HotStuffWaiter::<TPayload, TAddr>::new(
                identity,
                initial_committee,
                rx_new,
                rx_hs_message,
                rx_votes,
                tx_leader,
                tx_broadcast,
                tx_vote_message,
            )
            .run(rx_shutdown)
            .await
        })
    }

    pub fn new(
        identity: TAddr,
        initial_committee: Committee<TAddr>,
        rx_new: Receiver<TPayload>,
        rx_hs_message: Receiver<(TAddr, HotStuffMessage<TPayload>)>,
        rx_votes: Receiver<(TAddr, VoteMessage)>,
        tx_leader: Sender<HotStuffMessage<TPayload>>,
        tx_broadcast: Sender<(HotStuffMessage<TPayload>, Vec<TAddr>)>,
        tx_vote_message: Sender<(VoteMessage, TAddr)>,
    ) -> Self {
        Self {
            identity,
            committee: initial_committee,
            rx_new,
            rx_hs_message,
            rx_votes,
            tx_leader,
            tx_broadcast,
            tx_vote_message,
            shard_db: ShardDb::new(),
            current_payload: None,
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
        dbg!("on beat");
        if self.is_leader(payload.as_ref()) {
            // if self.current_payload.is_none() {
            self.current_payload = payload.clone();
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
    ) -> Result<HotStuffTreeNode<TPayload>, String> {
        dbg!("on propose");
        let qc = self.shard_db.get_high_qc_for(shard);

        let leaf_node = self.create_leaf(leaf, payload, qc, leaf_height + 1);
        self.shard_db.save_node(leaf_node.clone());
        self.tx_broadcast
            .send((
                HotStuffMessage::generic(leaf_node.clone(), shard),
                self.committee.members.clone(),
            ))
            .await
            .unwrap();
        Ok(leaf_node)
    }

    fn create_leaf(
        &self,
        parent: TreeNodeHash,
        payload: Option<TPayload>,
        qc: QuorumCertificate,
        height: u32,
    ) -> HotStuffTreeNode<TPayload> {
        HotStuffTreeNode::new(parent, payload, height, qc)
    }

    fn is_leader(&self, payload: Option<&TPayload>) -> bool {
        // TODO: determine actual leader
        true
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

    async fn on_receive_proposal(&mut self, from: TAddr, message: HotStuffMessage<TPayload>) -> Result<(), String> {
        if let Some(node) = message.node() {
            // TODO: validate message from leader
            let shard = message.shard();
            self.shard_db.save_node(node.clone());
            let v_height = self.shard_db.get_last_voted_height(shard);
            let (locked_node, locked_height) = self.shard_db.get_locked_node_hash_and_height(shard);

            dbg!(&locked_node);
            dbg!(locked_height);
            dbg!(node.height());
            dbg!(node.parent());

            // TODO: Change parent check to allow a chain?
            if node.height() > v_height &&
                (node.parent() == &locked_node || node.justify().node_height() > locked_height)
            {
                dbg!("Can vote on the message");
                self.shard_db.set_last_voted_height(shard, node.height());
                self.tx_vote_message
                    .send((
                        VoteMessage {
                            node_hash: node.hash().clone(),
                            shard,
                            signature: ValidatorSignature::from_bytes(&self.sign(node.hash(), shard)),
                        },
                        self.get_leader(),
                    ))
                    .await
                    .map_err(|e| e.to_string())?;
            } else {
                dbg!("Invalid proposal");
                dbg!("ignoring");
            }
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

    async fn on_receive_vote(&mut self, from: TAddr, msg: VoteMessage) -> Result<(), String> {
        // TODO: Only do this if you're the leader
        if self.shard_db.has_vote_for(&from, msg.node_hash.clone(), msg.shard) {
            return Ok(());
        }
        let total_votes = self
            .shard_db
            .save_vote_for(from, msg.node_hash.clone(), msg.shard, msg.signature);
        // Check for consensus
        dbg!(total_votes);
        dbg!(self.committee.consensus_threshold());
        if total_votes >= self.committee.consensus_threshold() {
            let signatures = self.shard_db.get_signatures_for(msg.node_hash.clone(), msg.shard);
            let node = self
                .shard_db
                .node(&msg.node_hash)
                .ok_or("Could not find node, was it saved previously?".to_string())
                .expect("should have been saved?");

            let qc = QuorumCertificate::new(
                HotStuffMessageType::Generic,
                node.height(),
                msg.node_hash,
                msg.shard,
                signatures.iter().map(|(_, sig)| sig.clone()).collect(),
            );
            dbg!(&qc);
            self.shard_db.update_high_qc(qc);
            // Should be the pace maker actually
            self.on_beat(msg.shard, None).await?;
        }
        Ok(())
    }

    fn get_leader(&self) -> TAddr {
        // currently I am the leader
        self.identity.clone()
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
                        dbg!("Hotstuff received");
                        dbg!(&msg);
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
                                dbg!("Generic message received");
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
    let instance = HotStuffWaiter::<String, String>::spawn(
        "leader".to_string(),
        Committee::empty(),
        rx_new,
        rx_hs_messages,
        rx_votes,
        tx_leader,
        tx_broadcast,
        tx_vote_message,
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
    let (tx_shutdown, rx_shutdown) = channel(1);
    let node1 = "node1".to_string();
    let node2 = "node2".to_string();
    let committee = Committee::new(vec![node1.clone(), node2.clone()]);

    let instance = HotStuffWaiter::<String, String>::spawn(
        node1.clone(),
        committee,
        rx_new,
        rx_hs_messages,
        rx_votes,
        tx_leader,
        tx_broadcast,
        tx_vote_message,
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
    let (tx_shutdown, rx_shutdown) = channel(1);
    let node1 = "node1".to_string();
    let node2 = "node2".to_string();
    let committee = Committee::new(vec![node1.clone(), node2.clone()]);

    let instance = HotStuffWaiter::<String, String>::spawn(
        node1.clone(),
        committee,
        rx_new,
        rx_hs_messages,
        rx_votes,
        tx_leader,
        tx_broadcast,
        tx_vote_message,
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
    let (tx_shutdown, rx_shutdown) = channel(1);
    let node1 = "node1".to_string();
    let node2 = "node2".to_string();
    let committee = Committee::new(vec![node1.clone(), node2.clone()]);

    let instance = HotStuffWaiter::<String, String>::spawn(
        node1.clone(),
        committee,
        rx_new,
        rx_hs_messages,
        rx_votes,
        tx_leader,
        tx_broadcast,
        tx_vote_message,
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
