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

use futures::future::join_all;
use log::*;
use tari_dan_common_types::{Epoch, PayloadId, ShardId, SubstateState};
use tari_engine_types::commit_result::{FinalizeResult, TransactionResult};
use tari_shutdown::ShutdownSignal;
use tokio::{
    sync::{
        broadcast,
        mpsc::{Receiver, Sender},
    },
    task::JoinHandle,
};

use crate::{
    consensus_constants::ConsensusConstants,
    models::{
        vote_message::VoteMessage,
        HotStuffMessage,
        HotStuffMessageType,
        HotStuffTreeNode,
        NodeHeight,
        ObjectPledge,
        Payload,
        QuorumCertificate,
        ShardVote,
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
    workers::{events::HotStuffEvent, hotstuff_error::HotStuffError},
};

const LOG_TARGET: &str = "tari::dan_layer::hotstuff_waiter";

pub struct HotStuffWaiter<TPayload, TAddr, TLeaderStrategy, TEpochManager, TPayloadProcessor, TShardStore> {
    identity: TAddr,
    leader_strategy: TLeaderStrategy,
    epoch_manager: TEpochManager,
    rx_new: Receiver<(TPayload, ShardId)>,
    rx_hs_message: Receiver<(TAddr, HotStuffMessage<TPayload, TAddr>)>,
    rx_votes: Receiver<(TAddr, VoteMessage)>,
    tx_leader: Sender<(TAddr, HotStuffMessage<TPayload, TAddr>)>,
    tx_broadcast: Sender<(HotStuffMessage<TPayload, TAddr>, Vec<TAddr>)>,
    tx_vote_message: Sender<(VoteMessage, TAddr)>,
    tx_events: broadcast::Sender<HotStuffEvent>,
    payload_processor: TPayloadProcessor,
    shard_store: TShardStore,
    consensus_constants: ConsensusConstants,
}

impl<TPayload, TAddr, TLeaderStrategy, TEpochManager, TPayloadProcessor, TShardStore>
    HotStuffWaiter<TPayload, TAddr, TLeaderStrategy, TEpochManager, TPayloadProcessor, TShardStore>
where
    TPayload: Payload + 'static,
    TAddr: NodeAddressable + 'static,
    TLeaderStrategy: LeaderStrategy<TAddr> + 'static + Send + Sync,
    TEpochManager: EpochManager<TAddr> + 'static + Send + Sync,
    TPayloadProcessor: PayloadProcessor<TPayload> + 'static + Send + Sync,
    TShardStore: ShardStoreFactory<Addr = TAddr, Payload = TPayload> + 'static + Send + Sync,
{
    pub fn spawn(
        identity: TAddr,
        epoch_manager: TEpochManager,
        leader_strategy: TLeaderStrategy,
        rx_new: Receiver<(TPayload, ShardId)>,
        rx_hs_message: Receiver<(TAddr, HotStuffMessage<TPayload, TAddr>)>,
        rx_votes: Receiver<(TAddr, VoteMessage)>,
        tx_leader: Sender<(TAddr, HotStuffMessage<TPayload, TAddr>)>,
        tx_broadcast: Sender<(HotStuffMessage<TPayload, TAddr>, Vec<TAddr>)>,
        tx_vote_message: Sender<(VoteMessage, TAddr)>,
        tx_events: broadcast::Sender<HotStuffEvent>,
        payload_processor: TPayloadProcessor,
        shard_store: TShardStore,
        shutdown: ShutdownSignal,
        consensus_constants: ConsensusConstants,
    ) -> JoinHandle<Result<(), HotStuffError>> {
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
            tx_events,
            payload_processor,
            shard_store,
            consensus_constants,
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
        tx_leader: Sender<(TAddr, HotStuffMessage<TPayload, TAddr>)>,
        tx_broadcast: Sender<(HotStuffMessage<TPayload, TAddr>, Vec<TAddr>)>,
        tx_vote_message: Sender<(VoteMessage, TAddr)>,
        tx_events: broadcast::Sender<HotStuffEvent>,
        payload_processor: TPayloadProcessor,
        shard_store: TShardStore,
        consensus_constants: ConsensusConstants,
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
            tx_events,
            payload_processor,
            shard_store,
            consensus_constants,
        }
    }

    // pacemaker
    async fn on_receive_new_view(
        &mut self,
        from: TAddr,
        shard: ShardId,
        qc: QuorumCertificate,
        payload: TPayload,
    ) -> Result<(), HotStuffError> {
        info!(
            target: LOG_TARGET,
            "🔥 Receive NewView for payload {} and shard {}",
            payload.to_id(),
            shard
        );
        // TODO: Validate who message is from
        let epoch = self.epoch_manager.current_epoch().await?;
        self.validate_from_committee(&from, epoch, shard).await?;
        self.validate_qc(&qc)?;
        let mut tx = self.shard_store.create_tx()?;
        tx.update_high_qc(shard, qc)
            .map_err(|e| HotStuffError::UpdateHighQcError(e.to_string()))?;
        tx.set_payload(payload).map_err(|e| e.into())?;
        tx.commit().map_err(|e| e.into())?;
        Ok(())
    }

    // pacemaker
    async fn on_beat(&mut self, shard: ShardId, payload_id: PayloadId) -> Result<(), HotStuffError> {
        // TODO: the leader is only known after the leaf is determines
        // TODO: Review if this is correct. The epoch should stay the same for all epochs
        let epoch = self.epoch_manager.current_epoch().await?;
        if self.is_leader(payload_id, shard, epoch).await? {
            self.on_propose(shard, payload_id).await?;
        }
        Ok(())
    }

    async fn on_propose(&mut self, shard: ShardId, payload_id: PayloadId) -> Result<(), HotStuffError> {
        info!(
            target: LOG_TARGET,
            "🔥 OnPropose for payload {} and shard {}", payload_id, shard
        );

        let epoch = self.epoch_manager.current_epoch().await?;

        let leaf_node;
        let qc;
        let actual_payload;
        let leaf;
        let leaf_height;
        {
            let tx = self.shard_store.create_tx()?;

            (leaf, leaf_height) = tx.get_leaf_node(shard).map_err(|e| e.into())?;
            qc = tx.get_high_qc_for(shard).map_err(|e| e.into())?;
            actual_payload = tx.get_payload(&payload_id).map_err(|e| e.into())?;
        }

        let involved_shards = actual_payload.involved_shards();
        let members = self
            .epoch_manager
            .get_committees(epoch, &involved_shards)
            .await?
            .into_iter()
            .flat_map(|allocation| allocation.committee.map(|c| c.members).unwrap_or_default())
            .collect();
        {
            let mut tx = self.shard_store.create_tx()?;

            let parent = tx.get_node(&leaf).map_err(|e| e.into())?;

            if leaf != TreeNodeHash::zero() && parent.justify().local_node_hash() == qc.local_node_hash() {
                info!(
                    target: LOG_TARGET,
                    "Leaf node already has the same QC as the proposed QC, so we have already sent a proposal for \
                     this payload"
                );
                // We have already sent a proposal for this paylaod. Do nothing
                return Ok(());
            }

            let payload_height = if parent.payload_id() == payload_id {
                parent.payload_height() + NodeHeight(1)
            } else {
                NodeHeight(0)
            };


            if payload_height > NodeHeight(self.consensus_constants.hotstuff_rounds) {
                info!(
                    target: LOG_TARGET,
                    "🔥 OnPropose payload {} and shard {} has height {}, this node has already been committed",
                    payload_id,
                    shard,
                    payload_height
                );

                // No need to continue, we have already committed this node.
                return Ok(());
            }
            let (change, claim) = actual_payload
                .objects_for_shard(shard)
                .ok_or(HotStuffError::ShardHasNoData)?;

            if !claim.is_valid(payload_id) {
                return Err(HotStuffError::ClaimIsNotValid);
            }
            let local_pledge = tx
                .pledge_object(shard, payload_id, change, leaf_height)
                .map_err(|e| e.into())?;
            info!(
                target: LOG_TARGET,
                "🔥 PLEDGE: {}",
                serde_json::to_string(&local_pledge).unwrap(),
            );
            leaf_node = self.create_leaf(
                leaf,
                shard,
                payload_id,
                if payload_height.as_u64() == 0 {
                    Some(actual_payload)
                } else {
                    None
                },
                qc,
                epoch,
                self.identity.clone(),
                NodeHeight(leaf_height.0 + 1),
                payload_height,
                vec![local_pledge],
            );
            tx.save_node(leaf_node.clone()).map_err(|e| e.into())?;
            tx.update_leaf_node(shard, *leaf_node.hash(), leaf_node.height())
                .map_err(|e| HotStuffError::UpdateLeafNode(e.to_string()))?;
            tx.commit().map_err(|e| e.into())?;
        }
        self.tx_broadcast
            .send((HotStuffMessage::generic(leaf_node.clone(), shard), members))
            .await
            .unwrap();
        Ok(())
    }

    fn create_leaf(
        &self,
        parent: TreeNodeHash,
        shard: ShardId,
        payload_id: PayloadId,
        payload: Option<TPayload>,
        qc: QuorumCertificate,
        epoch: Epoch,
        leader: TAddr,
        height: NodeHeight,
        payload_height: NodeHeight,
        local_pledges: Vec<ObjectPledge>,
    ) -> HotStuffTreeNode<TAddr, TPayload> {
        HotStuffTreeNode::new(
            parent,
            shard,
            height,
            payload_id,
            payload,
            payload_height,
            local_pledges,
            epoch,
            leader,
            qc,
        )
    }

    async fn is_leader(&self, payload: PayloadId, shard: ShardId, epoch: Epoch) -> Result<bool, HotStuffError> {
        Ok(self.leader_strategy.is_leader(
            &self.identity,
            &self.epoch_manager.get_committee(epoch, shard).await?,
            payload,
            shard,
            0,
        ))
    }

    async fn validate_from_committee(
        &mut self,
        from: &TAddr,
        epoch: Epoch,
        shard: ShardId,
    ) -> Result<(), HotStuffError> {
        if self.epoch_manager.get_committee(epoch, shard).await?.contains(from) {
            Ok(())
        } else {
            Err(HotStuffError::ReceivedMessageFromNonCommitteeMember)
        }
    }

    fn validate_qc(&self, _qc: &QuorumCertificate) -> Result<(), HotStuffError> {
        // TODO: get committee at epoch
        // TODO: Validate committee signatures
        Ok(())
    }

    async fn on_next_sync_view(&mut self, payload: TPayload, shard: ShardId) -> Result<(), HotStuffError> {
        info!(
            target: LOG_TARGET,
            "🔥 Send NEWVIEW for payload {} and shard {}",
            payload.to_id(),
            shard
        );
        let payload_id = payload.to_id();
        debug!(target: LOG_TARGET, "on_next_sync_view started: {:?}", payload_id);

        let new_view;
        {
            let mut tx = self.shard_store.create_tx()?;

            let high_qc = tx.get_high_qc_for(shard).map_err(|e| e.into())?;

            //  Save the payload, because we will need it when the proposal comes back
            tx.set_payload(payload.clone()).map_err(|e| e.into())?;
            tx.commit().map_err(|e| e.into())?;
            new_view = HotStuffMessage::new_view(high_qc, shard, Some(payload));
        }

        let epoch = self.epoch_manager.current_epoch().await?;
        let committee = self.epoch_manager.get_committee(epoch, shard).await?;
        let leader = self.leader_strategy.get_leader(&committee, payload_id, shard, 0);
        debug!(
            target: LOG_TARGET,
            "Determined leader for this payload is: {:?}, sending new view", leader
        );

        self.tx_leader
            .send((leader.clone(), new_view))
            .await
            .map_err(|_| HotStuffError::SendError)?;
        Ok(())
    }

    async fn update_nodes(
        &mut self,
        node: HotStuffTreeNode<TAddr, TPayload>,
        finalize_result: &FinalizeResult,
    ) -> Result<(), HotStuffError> {
        let shard = node.shard();

        if node.justify().local_node_hash() == TreeNodeHash::zero() {
            dbg!("Node is parented to genesis, no need to update");
            return Ok(());
        }

        let mut tx = self.shard_store.create_tx()?;
        tx.update_high_qc(shard, node.justify().clone())
            .map_err(|e| HotStuffError::UpdateHighQcError(e.to_string()))?;
        let b_two = tx.get_node(&node.justify().local_node_hash()).map_err(|e| e.into())?;

        if b_two.justify().local_node_hash() == TreeNodeHash::zero() {
            dbg!("b one is genesis, nothing to do");
            return Ok(());
        }
        let b_one = tx.get_node(&b_two.justify().local_node_hash()).map_err(|e| e.into())?;

        let (_b_lock, b_lock_height) = tx.get_locked_node_hash_and_height(shard).map_err(|e| e.into())?;
        if b_one.height() > b_lock_height {
            info!(target: LOG_TARGET, "Updating locked node to: {:?}", b_one.hash());
            tx.set_locked(shard, *b_one.hash(), b_one.height())
                .map_err(|e| e.into())?;
        }

        if node.justify().payload_height() == NodeHeight(self.consensus_constants.hotstuff_rounds - 2) {
            let result = finalize_result;
            self.publish_result_event(node.justify().payload_id(), result);
            let changes = extract_changes(node.payload_id(), result)?;
            info!(
                target: LOG_TARGET,
                "payload changeset: {}",
                serde_json::to_string(&changes).unwrap()
            );
            self.on_commit(node, &changes, &mut tx)?;
        }
        tx.commit().map_err(|e| e.into())?;

        Ok(())
    }

    fn publish_result_event(&self, payload_id: PayloadId, result: &FinalizeResult) {
        match result.result {
            TransactionResult::Accept(_) => {
                self.publish_event(HotStuffEvent::OnAccept(payload_id, result.clone()));
            },
            TransactionResult::Reject(ref reject) => {
                self.publish_event(HotStuffEvent::OnReject(payload_id, reject.clone()));
            },
        }
    }

    fn on_commit(
        &mut self,
        node: HotStuffTreeNode<TAddr, TPayload>,
        changes: &HashMap<ShardId, SubstateState>,
        tx: &mut TShardStore::Transaction,
    ) -> Result<(), HotStuffError> {
        let shard = node.shard();
        if tx.get_last_executed_height(shard).map_err(|e| e.into())? < node.height() {
            info!(
                target: LOG_TARGET,
                "🔥 OnCommit for payload {} and shard {}",
                node.payload_id(),
                shard,
            );

            if node.parent() != &TreeNodeHash::zero() {
                let parent = tx.get_node(node.parent()).map_err(|e| e.into())?;
                self.on_commit(parent, changes, tx)?;
            }

            if node.justify().payload_height() == NodeHeight(self.consensus_constants.hotstuff_rounds - 2) {
                tx.save_substate_changes(changes, &node).map_err(|e| e.into())?;
            }
            tx.set_last_executed_height(shard, node.height())
                .map_err(|e| e.into())?;
        }
        Ok(())
    }

    fn execute(
        &self,
        node: &HotStuffTreeNode<TAddr, TPayload>,
        payload: TPayload,
    ) -> Result<FinalizeResult, HotStuffError> {
        let shard_pledges = node
            .justify()
            .all_shard_nodes()
            .iter()
            .map(|s| (s.shard_id, s.pledges.clone()))
            .collect();
        let finalize = self.payload_processor.process_payload(payload, shard_pledges)?;
        Ok(finalize)
    }

    fn validate_proposal(&self, node: &HotStuffTreeNode<TAddr, TPayload>) -> Result<(), HotStuffError> {
        if node.payload_height() == NodeHeight(0) ||
            (node.payload_id() == node.justify().payload_id() &&
                node.payload_height() == node.justify().payload_height() + NodeHeight(1))
        {
            if node.payload_height() > NodeHeight(self.consensus_constants.hotstuff_rounds - 1) {
                return Err(HotStuffError::PayloadHeightIsTooHigh);
            }
            Ok(())
        } else {
            Err(HotStuffError::NodePayloadDoesNotMatchJustifyPayload)
        }
    }

    // TODO: needs some explanation of the process in docs here
    async fn on_receive_proposal(
        &mut self,
        from: TAddr,
        node: HotStuffTreeNode<TAddr, TPayload>,
    ) -> Result<(), HotStuffError> {
        info!(
            target: LOG_TARGET,
            "🔥 Receive PROPOSAL for payload {} and shard {} from {}",
            node.payload_id(),
            node.shard(),
            from,
        );

        // TODO: validate message from leader
        // TODO: Validate I am processing this shard
        // TODO: Validate the epoch is still valid
        self.validate_proposal(&node)?;

        let shard = node.shard();
        let payload = if let Some(node_payload) = node.payload() {
            node_payload.clone()
        } else {
            let tx = self.shard_store.create_tx()?;
            tx.get_payload(&node.payload_id()).map_err(|e| e.into())?
        };
        let involved_shards = payload.involved_shards();
        let local_shards = self
            .epoch_manager
            .filter_to_local_shards(node.epoch(), &self.identity, &involved_shards)
            .await?;

        let mut votes_to_send = vec![];
        // let mut finalize_result = None;

        // Execute the payload!
        let finalize_result = self.execute(&node, payload)?;

        {
            let mut tx = self.shard_store.create_tx()?;
            tx.save_node(node.clone()).map_err(|e| e.into())?;
            let v_height = tx.get_last_voted_height(shard).map_err(|e| e.into())?;
            // TODO: can also use the QC and committee to justify this....
            let (locked_node, locked_height) = tx.get_locked_node_hash_and_height(shard).map_err(|e| e.into())?;
            if node.height() > v_height &&
                (node.parent() == &locked_node || node.justify().local_node_height() > locked_height)
            {
                tx.save_leader_proposals(shard, node.payload_id(), node.payload_height(), node.clone())
                    .map_err(|e| e.into())?;

                let mut votes = vec![];
                for s in &involved_shards {
                    if let Some(vote) = tx
                        .get_leader_proposals(node.payload_id(), node.payload_height(), *s)
                        .map_err(|e| e.into())?
                    {
                        votes.push(ShardVote {
                            shard_id: *s,
                            node_hash: *vote.hash(),
                            pledges: vote.local_pledges().to_vec(),
                        });
                    } else {
                        break;
                    }
                }
                if votes.len() == involved_shards.len() {
                    // it may happen that we are involved in more than one committee, in which case send the votes to
                    // each leader.

                    for local_shard in local_shards {
                        dbg!("Can vote on the message");
                        let local_node = tx
                            .get_leader_proposals(node.payload_id(), node.payload_height(), local_shard)
                            .map_err(|e| e.into())?
                            .unwrap();

                        tx.set_last_voted_height(local_shard, local_node.height())
                            .map_err(|e| e.into())?;

                        let _signature = self.sign(node.hash(), shard);

                        let vote_msg = self.decide(*local_node.hash(), local_shard, votes.clone(), &finalize_result)?;

                        votes_to_send.push(self.tx_vote_message.send((
                            vote_msg,
                            local_node.proposed_by().clone(), // self.get_leader(),
                        )));
                    }

                    // finalize_result = Some(execution_result);
                } else {
                    info!(
                        target: LOG_TARGET,
                        "🔥 Not enough votes to vote on the message, votes: {}, involved_shards: {}",
                        votes.len(),
                        involved_shards.len()
                    );
                }
            } else {
                error!(target: LOG_TARGET, "Received a proposal that is not valid");
            }
            tx.commit().map_err(|e| e.into())?;
        }

        for res in join_all(votes_to_send).await {
            res.map_err(|_| HotStuffError::SendError)?;
        }
        self.update_nodes(node.clone(), &finalize_result).await?;
        Ok(())
    }

    fn sign(&self, _node_hash: &TreeNodeHash, _shard: ShardId) -> ValidatorSignature {
        // todo!();
        ValidatorSignature::from_bytes(&[]).unwrap()
    }

    fn decide(
        &self,
        local_node: TreeNodeHash,
        local_shard: ShardId,
        votes: Vec<ShardVote>,
        finalize_result: &FinalizeResult,
    ) -> Result<VoteMessage, HotStuffError> {
        let mut vote_msg = match finalize_result.result {
            TransactionResult::Accept(ref accept) => {
                info!(
                    target: LOG_TARGET,
                    "💚 Vote to ACCEPT payload. Up substate(s): {}, down substate(s): {}",
                    accept.up_iter().count(),
                    accept.down_iter().count(),
                );
                VoteMessage::accept(local_node, local_shard, votes)
            },
            TransactionResult::Reject(ref reject) => {
                info!(target: LOG_TARGET, "⚔ Vote to REJECT payload: {}", reject.reason);
                VoteMessage::reject(local_node, local_shard, votes)
            },
        };

        vote_msg.sign();
        Ok(vote_msg)
    }

    // The leader receives votes from his local shard, and forwards it to all other shards
    async fn on_receive_vote(&mut self, from: TAddr, msg: VoteMessage) -> Result<(), HotStuffError> {
        info!(
            target: LOG_TARGET,
            "🔥 Receive {:?} VOTE for shard {} from {}",
            msg.decision(),
            msg.shard(),
            from,
        );
        // TODO: Only do this if you're the leader
        let mut on_beat_future = None;
        let node;
        {
            let tx = self.shard_store.create_tx()?;
            if tx
                .has_vote_for(&from, msg.local_node_hash(), msg.shard())
                .map_err(|e| e.into())?
            {
                return Ok(());
            }

            node = tx.get_node(&msg.local_node_hash()).map_err(|e| e.into())?;

            if node.proposed_by() != &self.identity {
                return Err(HotStuffError::NotTheLeader);
            }
        }

        let valid_committee = self.epoch_manager.get_committee(node.epoch(), node.shard()).await?;
        {
            let mut tx = self.shard_store.create_tx()?;
            if !valid_committee.contains(&from) {
                return Err(HotStuffError::ReceivedMessageFromNonCommitteeMember);
            }

            let total_votes = tx
                .save_received_vote_for(from, msg.local_node_hash(), msg.shard(), msg.clone())
                .map_err(|e| e.into())?;
            // Check for consensus
            dbg!(&valid_committee);
            dbg!(total_votes);
            if total_votes >= valid_committee.consensus_threshold() {
                let mut different_votes = HashMap::new();
                for vote in tx
                    .get_received_votes_for(msg.local_node_hash(), msg.shard())
                    .map_err(|e| e.into())?
                {
                    let entry = different_votes.entry(vote.get_all_nodes_hash()).or_insert(vec![]);
                    entry.push(vote);
                }

                // Check that there is sufficient votes for a single set of nodes that we can use to generate a qc
                for (_hash, votes) in different_votes {
                    if votes.len() >= valid_committee.consensus_threshold() {
                        let signatures = votes.iter().map(|v| v.signature().clone()).collect();

                        let main_vote = votes.get(0).unwrap();

                        let qc = QuorumCertificate::new(
                            node.payload_id(),
                            node.payload_height(),
                            main_vote.local_node_hash(),
                            node.height(),
                            node.shard(),
                            node.epoch(),
                            main_vote.decision(),
                            main_vote.all_shard_nodes().clone(),
                            signatures,
                        );
                        tx.update_high_qc(msg.shard(), qc)
                            .map_err(|e| HotStuffError::UpdateHighQcError(e.to_string()))?; // TODO: is there a better alternative to handle error?

                        // Should be the pace maker actually
                        on_beat_future = Some(self.on_beat(msg.shard(), node.payload_id()));
                        break;
                    }
                }
            }

            // commit the transaction
            tx.commit().map_err(|e| e.into())?;
            // drop tx
        }

        if let Some(on_beat) = on_beat_future {
            on_beat.await?;
        }
        Ok(())
    }

    // fn get_leader(&self, payload: Option<&TPayload>, shard: u32) -> &TAddr {
    //     self.leader_strategy.get_leader(&self.committee, payload, shard)
    // }

    async fn on_new_hs_message(
        &mut self,
        from: TAddr,
        msg: HotStuffMessage<TPayload, TAddr>,
    ) -> Result<(), HotStuffError> {
        match msg.message_type() {
            HotStuffMessageType::NewView => {
                if let Some(payload) = msg.new_view_payload() {
                    self.on_receive_new_view(from, msg.shard(), msg.high_qc().unwrap(), payload.clone())
                        .await?;
                    // There should always be a payload, otherwise the leader
                    // can't be determined
                    self.on_beat(msg.shard(), payload.to_id()).await?;
                }
            },
            HotStuffMessageType::Generic => {
                let node = msg.node().ok_or(HotStuffError::RecvGenericMessageWithoutNode)?;
                self.on_receive_proposal(from, node.clone()).await?;
            },
        }
        Ok(())
    }

    fn publish_event(&self, event: HotStuffEvent) {
        let _ignore = self.tx_events.send(event);
    }

    pub async fn run(mut self, mut shutdown: ShutdownSignal) -> Result<(), HotStuffError> {
        loop {
            tokio::select! {
                msg = self.rx_new.recv() => {
                    if let Some((p, shard)) = msg {
                        if let Err(e) = self.on_next_sync_view(p, shard).await {
                           error!(target: LOG_TARGET, "Error while processing new payload (on_next_sync_view): {}", e);
                        }
                        // self.on_beat(0, msg);
                        // TODO: Start timer for receiving proposal
                    } else {
                        dbg!("All senders have dropped");
                        break;
                    }
                },
                Some((from, msg)) = self.rx_hs_message.recv() => {
                    if let Err(e) = self.on_new_hs_message(from, msg).await {
                        self.publish_event(HotStuffEvent::Failed(e.to_string()));
                        error!(target: LOG_TARGET, "Error while processing new hotstuff message (on_new_hs_message): {}", e);
                    }
                },
                Some((from, msg)) = self.rx_votes.recv() => {
                    debug!(target: LOG_TARGET, "Received vote from {}", from);
                    if let Err(e) = self.on_receive_vote(from, msg).await {
                        error!(target: LOG_TARGET, "Error while processing vote (on_receive_vote): {}", e);
                    }
                },
                _ = shutdown.wait() => {
                    info!(target: LOG_TARGET, "💤 Shutting down");
                    break;
                }
            }
        }
        Ok(())
    }
}

fn extract_changes(
    payload_id: PayloadId,
    finalize: &FinalizeResult,
) -> Result<HashMap<ShardId, SubstateState>, HotStuffError> {
    let mut changes = HashMap::new();
    match finalize.result {
        TransactionResult::Accept(ref diff) => {
            changes.extend(
                diff.up_iter()
                    .map(|(shard, substate)| {
                        (shard.into_shard_id().into(), SubstateState::Up {
                            created_by: payload_id,
                            data: substate.clone(),
                        })
                    })
                    .chain(diff.down_iter().map(|shard| {
                        (shard.into_shard_id().into(), SubstateState::Down {
                            deleted_by: payload_id,
                        })
                    })),
            );
        },
        TransactionResult::Reject(ref reject) => return Err(HotStuffError::TransactionRejected(reject.reason.clone())),
    }

    Ok(changes)
}
