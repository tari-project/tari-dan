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

use std::collections::HashMap;

use tari_dan_common_types::{PayloadId, ShardId, SubstateState};
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
        PayloadProcessor,
    },
    storage::shard_store::{ShardStoreFactory, ShardStoreTransaction},
};

pub struct HotStuffWaiter<
    TPayload: Payload,
    TAddr: NodeAddressable,
    TLeaderStrategy: LeaderStrategy<TAddr>,
    TEpochManager: EpochManager<TAddr>,
    TPayloadProcessor: PayloadProcessor<TPayload>,
    TShardStore: ShardStoreFactory,
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
    payload_processor: TPayloadProcessor,
    shard_store: TShardStore,
}

impl<
        TPayload: Payload + 'static,
        TAddr: NodeAddressable + 'static,
        TLeaderStrategy: LeaderStrategy<TAddr> + 'static + Send + Sync,
        TEpochManager: EpochManager<TAddr> + 'static + Send + Sync,
        TPayloadProcessor: PayloadProcessor<TPayload> + 'static + Send + Sync,
        TShardStore: ShardStoreFactory<Addr = TAddr, Payload = TPayload> + 'static + Send + Sync,
    > HotStuffWaiter<TPayload, TAddr, TLeaderStrategy, TEpochManager, TPayloadProcessor, TShardStore>
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
        payload_processor: TPayloadProcessor,
        shard_store: TShardStore,
        shutdown: ShutdownSignal,
    ) -> JoinHandle<Result<(), String>> {
        let waiter = HotStuffWaiter::new(
            identity,
            epoch_manager,
            leader_strategy,
            rx_new,
            rx_hs_message,
            rx_votes,
            tx_leader,
            tx_broadcast,
            tx_vote_message,
            payload_processor,
            shard_store,
        );
        tokio::spawn(waiter.run(shutdown))
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
        payload_processor: TPayloadProcessor,
        shard_store: TShardStore,
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
            payload_processor,
            shard_store,
        }
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
        let epoch = self.epoch_manager.current_epoch();
        self.validate_from_committee(&from, epoch, shard).await?;
        self.validate_qc(&qc)?;
        let mut tx = self.shard_store.create_tx();
        tx.update_high_qc(shard, qc);
        tx.set_payload(payload);
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    }

    // pacemaker
    async fn on_beat(&mut self, shard: ShardId, payload: PayloadId) -> Result<(), String> {
        // TODO: the leader is only known after the leaf is determines
        // TODO: Review if this is correct. The epoch should stay the same for all epochs
        let epoch = self.epoch_manager.current_epoch();
        if self.is_leader(payload, shard, epoch).await? {
            self.on_propose(shard, payload).await?;
        }
        Ok(())
    }

    async fn on_propose(&mut self, shard: ShardId, payload: PayloadId) -> Result<HotStuffTreeNode<TAddr>, String> {
        dbg!(&self.identity, "on propose");

        let epoch = self.epoch_manager.current_epoch();

        let leaf_node;
        let members;
        {
            let mut tx = self.shard_store.create_tx();

            let (leaf, leaf_height) = tx.get_leaf_node(shard);
            let qc = tx.get_high_qc_for(shard);
            let actual_payload = tx.get_payload(&payload).ok_or("Could not find payload")?;

            let involved_shards = actual_payload.involved_shards();
            members = self
                .epoch_manager
                .get_committees(epoch, &involved_shards)?
                .into_iter()
                .flat_map(|allocation| allocation.committee.map(|c| c.members).unwrap_or_default())
                .collect();

            let parent = tx.get_node(&leaf).ok_or("Could not find leaf")?;

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
                local_pledges.push(tx.pledge_object(shard, object, change, payload, leaf_height));
            }
            leaf_node = self.create_leaf(
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
            tx.save_node(leaf_node.clone());
            tx.update_leaf_node(shard, *leaf_node.hash(), leaf_node.height())?;
            tx.commit().map_err(|e| e.to_string())?;
        }
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

    async fn is_leader(&self, payload: PayloadId, shard: ShardId, epoch: Epoch) -> Result<bool, String> {
        Ok(self.leader_strategy.is_leader(
            &self.identity,
            &self.epoch_manager.get_committee(epoch, shard)?,
            payload,
            shard,
            0,
        ))
    }

    async fn validate_from_committee(&mut self, from: &TAddr, epoch: Epoch, shard: ShardId) -> Result<(), String> {
        if self.epoch_manager.get_committee(epoch, shard)?.contains(from) {
            Ok(())
        } else {
            Err("From is not part of this committee".to_string())
        }
    }

    fn validate_qc(&self, _qc: &QuorumCertificate) -> Result<(), String> {
        // TODO: get committee at epoch
        // TODO: Validate committee signatures
        Ok(())
    }

    async fn on_next_sync_view(&mut self, payload: TPayload, shard: ShardId) -> Result<(), String> {
        dbg!("new payload received", &shard);

        let new_view;
        {
            let tx = self.shard_store.create_tx();

            let high_qc = tx.get_high_qc_for(shard);

            new_view = HotStuffMessage::new_view(high_qc, shard, Some(payload));
        }
        self.tx_leader.send(new_view).await.map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn update_nodes(&mut self, node: HotStuffTreeNode<TAddr>, shard: ShardId) -> Result<(), String> {
        let mut tx = self.shard_store.create_tx();
        if node.justify().local_node_hash() == TreeNodeHash::zero() {
            dbg!("Node is parented to genesis, no need to update");
            return Ok(());
        }
        tx.update_high_qc(shard, node.justify().clone());
        let b_two = tx.get_node(&node.justify().local_node_hash()).ok_or("No node b2")?;

        if b_two.justify().local_node_hash() == TreeNodeHash::zero() {
            dbg!("b one is genesis, nothing to do");
            return Ok(());
        }
        let b_one = tx.get_node(&b_two.justify().local_node_hash()).ok_or("No node b1")?;

        let (_b_lock, b_lock_height) = tx.get_locked_node_hash_and_height(shard);
        if b_one.height().0 > b_lock_height.0 {
            // commit
            dbg!("Commiting height", b_one.height());
            tx.set_locked(shard, *b_one.hash(), b_one.height());
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
            self.on_commit(node, shard, &mut tx)?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    }

    fn on_commit(
        &mut self,
        node: HotStuffTreeNode<TAddr>,
        shard: ShardId,
        tx: &mut TShardStore::Transaction,
    ) -> Result<(), String> {
        if tx.get_last_executed_height(shard) < node.height() {
            if node.parent() != &TreeNodeHash::zero() {
                let parent = tx.get_node(node.parent()).ok_or("No parent node")?;
                dbg!("Committing parent");
                self.on_commit(parent, shard, tx)?;
            }
            if node.justify().payload_height() == NodeHeight(2) {
                let payload = tx.get_payload(&node.justify().payload()).ok_or("No payload")?;

                let mut all_pledges = HashMap::new();
                for (pledge_shard, _, pledges) in node.justify().all_shard_nodes() {
                    all_pledges.insert(*pledge_shard, pledges.clone());
                }
                let changes = self.execute(all_pledges, payload)?;
                tx.save_substate_changes(changes, *node.hash());
            }
            tx.set_last_executed_height(shard, node.height());
        }
        Ok(())
    }

    fn execute(
        &mut self,
        shard_pledges: HashMap<ShardId, Vec<ObjectPledge>>,
        payload: TPayload,
    ) -> Result<HashMap<ShardId, Option<SubstateState>>, String> {
        self.payload_processor
            .process_payload(&payload, shard_pledges)
            .map_err(|e| e.to_string())?;
        // let (reply_tx, reply_rx) = oneshot::channel();
        // self.tx_execute
        //     .send((payload, shard_pledges, reply_tx))
        //     .await
        //     .map_err(|e| format!("Could not send execute cmd:{}", e))?;
        // TODO: wait on results
        // let result = reply_rx
        //     .await
        //     .map_err(|e| format!("Could not receive execute reply:{}", e))?;
        Ok(HashMap::new())
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
        let mut votes_to_send = vec![];
        {
            let mut tx = self.shard_store.create_tx();
            tx.save_node(node.clone());
            let v_height = tx.get_last_voted_height(shard);
            // TODO: can also use the QC and committee to justify this....
            let (locked_node, locked_height) = tx.get_locked_node_hash_and_height(shard);
            if node.height() > v_height &&
                (node.parent() == &locked_node || node.justify().local_node_height() > locked_height)
            {
                tx.save_payload_vote(shard, node.payload(), node.payload_height(), node.clone());

                let payload = tx.get_payload(&node.payload()).ok_or("No payload found")?;
                let involved_shards = payload.involved_shards();
                let mut votes = vec![];
                for s in &involved_shards {
                    if let Some(vote) = tx.get_payload_vote(node.payload(), node.payload_height(), *s) {
                        votes.push((*s, *vote.hash(), vote.local_pledges().to_vec()));
                    } else {
                        break;
                    }
                }
                dbg!(&self.identity, "Votes received", votes.len());
                if votes.len() == involved_shards.len() {
                    let local_shards = self
                        .epoch_manager
                        .get_shards(node.epoch(), &self.identity, &involved_shards)?;
                    // it may happen that we are involved in more than one committee, in which case send the votes to
                    // each leader.

                    for local_shard in local_shards {
                        dbg!("Can vote on the message");
                        let local_node = tx
                            .get_payload_vote(node.payload(), node.payload_height(), local_shard)
                            .unwrap();

                        tx.set_last_voted_height(local_shard, local_node.height());

                        let _signature = ValidatorSignature::from_bytes(&self.sign(node.hash(), shard));
                        // TODO: Actually decide on this
                        let decision = QuorumDecision::Accept;
                        let mut vote_msg = VoteMessage::new(*local_node.hash(), local_shard, decision, votes.clone());
                        vote_msg.sign();

                        tx.commit().map_err(|e| e.to_string())?;
                        votes_to_send.push(self.tx_vote_message.send((
                            vote_msg,
                            local_node.proposed_by().clone(), // self.get_leader(),
                        )));
                        // .await
                        // .map_err(|e| e.to_string())?;
                    }
                }
            } else {
                dbg!("Invalid proposal");
                dbg!("ignoring");
            }
        }
        for vote in votes_to_send {
            vote.await.map_err(|e| e.to_string())?;
        }
        self.update_nodes(node.clone(), shard).await?;
        Ok(())
    }

    fn sign(&self, _node_hash: &TreeNodeHash, _shard: ShardId) -> Vec<u8> {
        // todo!();
        vec![]
    }

    // The leader receives votes from his local shard, and forwards it to all other shards
    async fn on_receive_vote(&mut self, from: TAddr, msg: VoteMessage) -> Result<(), String> {
        // TODO: Only do this if you're the leader
        let mut on_beat_future = None;
        {
            let mut tx = self.shard_store.create_tx();
            if tx.has_vote_for(&from, msg.local_node_hash(), msg.shard()) {
                return Ok(());
            }

            let node = tx
                .get_node(&msg.local_node_hash())
                .ok_or("Could not find node, was it saved previously?")
                .expect("should have been saved?");

            if node.proposed_by() != &self.identity {
                return Err("I am not the leader for this node".to_string());
            }

            let valid_committee = self.epoch_manager.get_committee(node.epoch(), node.shard())?;

            if !valid_committee.contains(&from) {
                return Err("Not a valid committee member".to_string());
            }

            let total_votes = tx.save_received_vote_for(from, msg.local_node_hash(), msg.shard(), msg.clone());
            // Check for consensus
            dbg!(total_votes);
            if total_votes >= valid_committee.consensus_threshold() {
                let mut different_votes = HashMap::new();
                for vote in tx.get_received_votes_for(msg.local_node_hash(), msg.shard()) {
                    let entry = different_votes.entry(vote.get_all_nodes_hash()).or_insert(vec![]);
                    entry.push(vote);
                }

                // Check that there is sufficient votes for a single set of nodes that we can use to generate a qc
                for (_hash, votes) in different_votes {
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
                            main_vote.all_shard_nodes().clone(),
                            signatures,
                        );
                        tx.update_high_qc(msg.shard(), qc);
                        tx.commit().map_err(|e| e.to_string())?;
                        // Should be the pace maker actually
                        on_beat_future = Some(self.on_beat(msg.shard(), node.payload()));
                        break;
                    }
                    dbg!("Not enough votes for this one", votes.len());
                }
                dbg!("Enough votes, but not enough for a single node");
            }
        }
        if let Some(on_beat) = on_beat_future {
            on_beat.await?;
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
                                    self.on_receive_new_view(from, msg.shard(), msg.high_qc().unwrap(), payload.clone()).await?;
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
