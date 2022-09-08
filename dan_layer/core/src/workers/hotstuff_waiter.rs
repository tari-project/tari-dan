use std::collections::HashMap;

use async_recursion::async_recursion;
use tari_dan_common_types::{PayloadId, ShardId};
use tari_shutdown::ShutdownSignal;
use tokio::{
    sync::mpsc::{Receiver, Sender},
    task::JoinHandle,
};

use crate::{
    models::{
        vote_message::VoteMessage,
        Epoch,
        HotStuffMessage,
        HotStuffMessageType,
        HotStuffTreeNode,
        NodeHeight,
        ObjectPledge,
        Payload,
        QuorumCertificate,
        QuorumDecision,
        TreeNodeHash,
        ValidatorSignature,
    },
    services::{
        epoch_manager::EpochManager,
        infrastructure_services::NodeAddressable,
        leader_strategy::LeaderStrategy,
    },
    storage::shard_db::ShardDb,
};

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
        TLeaderStrategy: LeaderStrategy<TAddr, TPayload> + 'static + Send + Sync,
        TEpochManager: EpochManager<TAddr> + 'static + Send + Sync,
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
        shutdown: ShutdownSignal,
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
            .run(shutdown)
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
    async fn on_receive_new_view(
        &mut self,
        from: TAddr,
        shard: ShardId,
        qc: QuorumCertificate,
        payload: TPayload,
    ) -> Result<(), String> {
        // TODO: Validate who message is from
        let epoch = self.epoch_manager.current_epoch().await;
        self.validate_from_committee(&from, epoch, shard).await?;
        self.validate_qc(&qc);
        self.shard_db.update_high_qc(qc);
        self.shard_db.set_payload(payload);
        Ok(())
    }

    // pacemaker
    async fn on_beat(&mut self, shard: ShardId, payload: PayloadId) -> Result<(), String> {
        // TODO: the leader is only known after the leaf is determines
        // TODO: Review if this is correct. The epoch should stay the same for all epochs
        let epoch = self.epoch_manager.current_epoch().await;
        if self.is_leader(payload, shard, epoch).await? {
            dbg!(&self.identity, "I am the leader");
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
        dbg!(&self.identity, "on propose");
        let qc = self.shard_db.get_high_qc_for(shard);
        let epoch = self.epoch_manager.current_epoch().await;
        let actual_payload = self
            .shard_db
            .get_payload(&payload)
            .ok_or("Could not find payload".to_string())?;
        let involved_shards = actual_payload.involved_shards().to_vec();
        let members = self
            .epoch_manager
            .get_committees(epoch, &involved_shards)
            .await?
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

    async fn is_leader(&mut self, payload: PayloadId, shard: ShardId, epoch: Epoch) -> Result<bool, String> {
        Ok(self.leader_strategy.is_leader(
            &self.identity,
            &self.epoch_manager.get_committee(epoch, shard).await?,
            payload,
            shard,
            0,
        ))
    }

    async fn validate_from_committee(&mut self, from: &TAddr, epoch: Epoch, shard: ShardId) -> Result<(), String> {
        if self.epoch_manager.get_committee(epoch, shard).await?.contains(from) {
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
        dbg!("new payload received", &shard);

        // get state
        let high_qc = self.get_highest_qc(shard);
        // send to leader

        let new_view = HotStuffMessage::new_view(high_qc, shard, Some(payload));

        self.tx_leader.send(new_view).await.map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn update_nodes(&mut self, node: HotStuffTreeNode<TAddr>, shard: ShardId) -> Result<(), String> {
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
        let b_one = self
            .shard_db
            .node(&b_two.justify().local_node_hash())
            .ok_or("No node b1")?
            .clone();

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
        if node.justify().payload_height() == NodeHeight(2) {
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
            if node.justify().payload_height() == NodeHeight(2) {
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
        dbg!("Received proposal", &self.identity, &from);
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
            self.shard_db
                .save_payload_vote(shard, node.payload(), node.payload_height(), node.clone());

            let payload = self
                .shard_db
                .get_payload(&node.payload())
                .ok_or("No payload found".to_string())?;
            let involved_shards = payload.involved_shards();
            let mut votes = vec![];
            for s in &involved_shards {
                if let Some(vote) = self
                    .shard_db
                    .get_payload_vote(node.payload(), node.payload_height(), *s)
                {
                    votes.push((*s, vote.hash().clone(), vote.local_pledges().to_vec()));
                } else {
                    break;
                }
            }
            dbg!(&self.identity, "Votes recieved", votes.len());
            if votes.len() == involved_shards.len() {
                let local_shards = self
                    .epoch_manager
                    .get_shards(node.epoch(), &self.identity, &involved_shards)
                    .await?;
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
                    // TODO: Actually decide on this
                    let decision = QuorumDecision::Accept;
                    let mut vote_msg =
                        VoteMessage::new(local_node.hash().clone(), local_shard, decision, votes.clone());
                    vote_msg.sign();

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
            .has_vote_for(&from, msg.local_node_hash().clone(), msg.shard())
        {
            return Ok(());
        }

        let node = self
            .shard_db
            .node(&msg.local_node_hash())
            .ok_or("Could not find node, was it saved previously?".to_string())
            .expect("should have been saved?");

        if node.proposed_by() != &self.identity {
            return Err("I am not the leader for this node".to_string());
        }

        let valid_committee = self.epoch_manager.get_committee(node.epoch(), node.shard()).await?;

        if !valid_committee.contains(&from) {
            return Err("Not a valid committee member".to_string());
        }

        let total_votes =
            self.shard_db
                .save_received_vote_for(from, msg.local_node_hash().clone(), msg.shard(), msg.clone());
        // Check for consensus
        dbg!(total_votes);
        if total_votes >= valid_committee.consensus_threshold() {
            let mut different_votes = HashMap::new();
            for vote in self
                .shard_db
                .get_received_votes_for(msg.local_node_hash().clone(), msg.shard())
            {
                let mut entry = different_votes.entry(vote.get_all_nodes_hash()).or_insert(vec![]);
                entry.push(vote);
            }

            // Check that there is sufficient votes for a single set of nodes that we can use to generate a qc
            for (hash, votes) in different_votes {
                if votes.len() >= valid_committee.consensus_threshold() {
                    let signatures = votes.iter().map(|v| v.signature().clone()).collect();

                    let main_vote = votes.get(0).unwrap();

                    let qc = QuorumCertificate::new(
                        node.payload(),
                        node.payload_height(),
                        main_vote.local_node_hash(),
                        node.height(),
                        node.shard(),
                        node.epoch(),
                        main_vote.decision(),
                        main_vote.other_shard_nodes().clone(),
                        signatures,
                    );
                    self.shard_db.update_high_qc(qc);
                    // Should be the pace maker actually
                    self.on_beat(msg.shard(), node.payload()).await?;
                    return Ok(());
                }
                dbg!("Not enough votes for this one", votes.len());
            }
            dbg!("Enough votes, but not enough for a single node");
        }
        Ok(())
    }

    // fn get_leader(&self, payload: Option<&TPayload>, shard: u32) -> &TAddr {
    //     self.leader_strategy.get_leader(&self.committee, payload, shard)
    // }

    pub async fn run(mut self, mut shutdown: ShutdownSignal) -> Result<(), String> {
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
                                    self.on_receive_new_view(from, msg.shard(), msg.high_qc().unwrap(), payload.clone()).await;
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
                _ = shutdown.wait() => {
                    dbg!("Exiting");
                    break;
                }
            }
        }
        Ok(())
    }
}
