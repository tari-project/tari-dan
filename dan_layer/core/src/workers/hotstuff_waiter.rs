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

use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

use log::*;
use rand::seq::SliceRandom;
use serde::Serialize;
use tari_common_types::types::{FixedHash, PublicKey, Signature};
use tari_core::{ValidatorNodeMmr, ValidatorNodeMmrHasherBlake256};
use tari_dan_common_types::{
    optional::Optional,
    Epoch,
    NodeAddressable,
    NodeHeight,
    ObjectPledge,
    PayloadId,
    QuorumCertificate,
    ShardId,
    ShardPledge,
    ShardPledgeCollection,
    SubstateState,
    TreeNodeHash,
};
use tari_dan_engine::runtime::ConsensusContext;
use tari_engine_types::{
    commit_result::{FinalizeResult, RejectReason, TransactionResult},
    substate::SubstateDiff,
};
use tari_mmr::leaf_index::LeafIndex;
use tari_shutdown::ShutdownSignal;
use tari_transaction::SubstateChange;
use tokio::{
    sync::{
        broadcast,
        mpsc::{Receiver, Sender},
    },
    task,
    task::JoinHandle,
};

use super::pacemaker_worker::PacemakerHandle;
use crate::{
    consensus_constants::ConsensusConstants,
    models::{
        vote_message::VoteMessage,
        Committee,
        HotStuffMessage,
        HotStuffMessageType,
        HotStuffTreeNode,
        HotstuffPhase,
        Payload,
        PayloadResult,
    },
    services::{epoch_manager::EpochManager, leader_strategy::LeaderStrategy, PayloadProcessor, SigningService},
    storage::shard_store::{ShardStore, ShardStoreReadTransaction, ShardStoreWriteTransaction},
    workers::{
        events::HotStuffEvent,
        hotstuff_error::{HotStuffError, ProposalValidationError},
    },
};

const LOG_TARGET: &str = "tari::dan_layer::hotstuff_waiter";
pub const NETWORK_LATENCY: Duration = Duration::from_secs(10);

// This is the value that we wait over in the pacemaker. So when it trigger we know what triggered it.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub enum PacemakerEvents {
    LocalCommittee(Epoch, ShardId, PayloadId),
    ForeignCommittee(Epoch, ShardId, PayloadId),
}

// Messages that are send between replicas in case of leader failure
#[derive(Debug, Serialize, Clone)]
pub enum RecoveryMessage {
    MissingProposal(Epoch, ShardId, PayloadId, NodeHeight),
    ElectionInProgress(Epoch, ShardId, PayloadId),
}

pub struct HotStuffWaiter<
    TPayload,
    TAddr,
    TLeaderStrategy,
    TEpochManager,
    TPayloadProcessor,
    TShardStore,
    TSigningService,
> {
    signing_service: TSigningService,
    public_key: TAddr,
    leader_strategy: TLeaderStrategy,
    /// The epoch manager
    epoch_manager: TEpochManager,
    /// Received payloads that should be proposed. Only payloads that involve this node are pushed on this channel.
    rx_new: Receiver<(TPayload, ShardId)>,
    /// Received replica hotstuff messages, namely Proposal messages from the leader or
    /// NewView messages from replicas.
    rx_hs_message: Receiver<(TAddr, HotStuffMessage<TPayload, TAddr>)>,
    /// Received replica recovery message, namely request for missing node, response election in progress.
    rx_recovery_message: Receiver<(TAddr, RecoveryMessage)>,
    /// Received vote messages
    rx_votes: Receiver<(TAddr, VoteMessage)>,
    /// Hotstuff messages that should be delivered to the leader
    tx_leader: Sender<(TAddr, HotStuffMessage<TPayload, TAddr>)>,
    /// Hotstuff messages that should be delivered to the replicas
    tx_broadcast: Sender<(HotStuffMessage<TPayload, TAddr>, Vec<TAddr>)>,
    /// Recovery messages that should be delivered to replica that initiated recovery
    tx_recovery: Sender<(RecoveryMessage, TAddr)>,
    /// Recovery messages that should be delivered to foreign committee
    tx_recovery_broadcast: Sender<(RecoveryMessage, Vec<TAddr>)>,
    /// Vote messages that should be delivered to the leader
    tx_vote_message: Sender<(VoteMessage, TAddr)>,
    /// HotstuffEvent channel
    tx_events: broadcast::Sender<HotStuffEvent>,
    /// The pacemaker handle. Provides an interface to request timing leader responses
    #[allow(dead_code)]
    pacemaker: PacemakerHandle<PacemakerEvents>,
    /// The payload processor. This determines whether a payload proposal results in an accepted or rejected vote.
    payload_processor: TPayloadProcessor,
    /// Store used to persist consensus state.
    shard_store: TShardStore,
    /// Network-wide constants
    // TODO: remove if not needed
    consensus_constants: ConsensusConstants,
    /// NEWVIEW message counts - TODO: this will bloat memory maybe moving to the db is better
    newview_message_counts: HashMap<(ShardId, PayloadId), HashSet<TAddr>>,
    /// Network latency
    #[allow(dead_code)]
    network_latency: Duration,
    /// We store what round is for next leader selection, default is 0.
    current_leader_round: HashMap<(ShardId, PayloadId), u32>,
    /// We have to store if we are in the middle of an election, so that when we receive a recovery message
    /// from another VN, we can answer that we are electing and it should wait.
    election_in_progress: HashSet<(ShardId, PayloadId)>,
}

impl<TPayload, TAddr, TLeaderStrategy, TEpochManager, TPayloadProcessor, TShardStore, TSigningService>
    HotStuffWaiter<TPayload, TAddr, TLeaderStrategy, TEpochManager, TPayloadProcessor, TShardStore, TSigningService>
where
    TPayload: Payload + 'static,
    TAddr: NodeAddressable + 'static,
    TLeaderStrategy: LeaderStrategy<TAddr> + 'static + Send + Sync,
    TEpochManager: EpochManager<TAddr> + 'static + Send + Sync,
    TPayloadProcessor: PayloadProcessor<TPayload> + 'static + Send + Sync,
    TShardStore: ShardStore<Addr = TAddr, Payload = TPayload> + 'static + Send + Sync,
    TSigningService: SigningService + Sync + Send + 'static,
{
    pub fn spawn(
        signing_service: TSigningService,
        public_key: TAddr,
        epoch_manager: TEpochManager,
        leader_strategy: TLeaderStrategy,
        rx_new: Receiver<(TPayload, ShardId)>,
        rx_hs_message: Receiver<(TAddr, HotStuffMessage<TPayload, TAddr>)>,
        rx_recovery_message: Receiver<(TAddr, RecoveryMessage)>,
        rx_votes: Receiver<(TAddr, VoteMessage)>,
        tx_leader: Sender<(TAddr, HotStuffMessage<TPayload, TAddr>)>,
        tx_broadcast: Sender<(HotStuffMessage<TPayload, TAddr>, Vec<TAddr>)>,
        tx_recovery: Sender<(RecoveryMessage, TAddr)>,
        tx_recovery_broadcast: Sender<(RecoveryMessage, Vec<TAddr>)>,
        tx_vote_message: Sender<(VoteMessage, TAddr)>,
        tx_events: broadcast::Sender<HotStuffEvent>,
        pacemaker: PacemakerHandle<PacemakerEvents>,
        payload_processor: TPayloadProcessor,
        shard_store: TShardStore,
        shutdown: ShutdownSignal,
        consensus_constants: ConsensusConstants,
        network_latency: Duration,
    ) -> JoinHandle<anyhow::Result<()>> {
        let waiter = HotStuffWaiter::new(
            signing_service,
            public_key,
            epoch_manager,
            leader_strategy,
            rx_new,
            rx_hs_message,
            rx_recovery_message,
            rx_votes,
            tx_leader,
            tx_broadcast,
            tx_recovery,
            tx_recovery_broadcast,
            tx_vote_message,
            tx_events,
            pacemaker,
            payload_processor,
            shard_store,
            consensus_constants,
            network_latency,
        );
        tokio::spawn(async move {
            waiter.run(shutdown).await?;
            Ok(())
        })
    }

    pub fn new(
        signing_service: TSigningService,
        public_key: TAddr,
        epoch_manager: TEpochManager,
        leader_strategy: TLeaderStrategy,
        rx_new: Receiver<(TPayload, ShardId)>,
        rx_hs_message: Receiver<(TAddr, HotStuffMessage<TPayload, TAddr>)>,
        rx_recovery_message: Receiver<(TAddr, RecoveryMessage)>,
        rx_votes: Receiver<(TAddr, VoteMessage)>,
        tx_leader: Sender<(TAddr, HotStuffMessage<TPayload, TAddr>)>,
        tx_broadcast: Sender<(HotStuffMessage<TPayload, TAddr>, Vec<TAddr>)>,
        tx_recovery: Sender<(RecoveryMessage, TAddr)>,
        tx_recovery_broadcast: Sender<(RecoveryMessage, Vec<TAddr>)>,
        tx_vote_message: Sender<(VoteMessage, TAddr)>,
        tx_events: broadcast::Sender<HotStuffEvent>,
        pacemaker: PacemakerHandle<PacemakerEvents>,
        payload_processor: TPayloadProcessor,
        shard_store: TShardStore,
        consensus_constants: ConsensusConstants,
        network_latency: Duration,
    ) -> Self {
        Self {
            signing_service,
            public_key,
            epoch_manager,
            leader_strategy,
            rx_new,
            rx_hs_message,
            rx_recovery_message,
            rx_votes,
            tx_leader,
            tx_broadcast,
            tx_recovery,
            tx_recovery_broadcast,
            tx_vote_message,
            tx_events,
            pacemaker,
            payload_processor,
            shard_store,
            consensus_constants,
            newview_message_counts: HashMap::new(),
            network_latency,
            current_leader_round: HashMap::new(),
            election_in_progress: HashSet::new(),
        }
    }

    /// Step 1: A new payload has been received. The payload is persisted and all nodes send a NEWVIEW to the leader.
    async fn on_next_sync_view(&mut self, payload: TPayload, shard: ShardId) -> Result<(), HotStuffError> {
        let epoch = self.epoch_manager.current_epoch().await?;

        let payload_id = payload.to_id();
        let involved_shards = payload.involved_shards();
        let local_shards = self
            .epoch_manager
            .filter_to_local_shards(epoch, &self.public_key, &involved_shards)
            .await?;
        debug!(target: LOG_TARGET, "on_next_sync_view started: {}", payload_id);

        let committee = self.epoch_manager.get_committee(epoch, shard).await?;
        // Here the current_leader is always zero.
        let current_leader = self.current_leader_round.entry((shard, payload_id)).or_default();
        let leader = self
            .leader_strategy
            .get_leader(&committee, payload_id, shard, *current_leader);

        let new_view = self.shard_store.with_write_tx(|tx| {
            let high_qc = tx
                .get_high_qc_for(payload_id, shard)
                .optional()?
                .unwrap_or_else(|| QuorumCertificate::genesis(epoch, payload_id, shard));

            //  Save the payload, because we will need it when the proposal comes back
            tx.save_payload(payload.clone())?;

            Ok::<_, HotStuffError>(HotStuffMessage::new_view(high_qc, shard, payload))
        })?;

        info!(
            target: LOG_TARGET,
            "👑 [epoch: {}] Leader for payload {} shard {} is: {:?}{}",
            epoch,
            payload_id,
            shard,
            leader,
            if *leader == self.public_key { " (this node)" } else { "" }
        );
        info!(
            target: LOG_TARGET,
            "🔥 Sending NEWVIEW with high qc {} {} to leader",
            new_view.high_qc().unwrap().node_height(),
            new_view.high_qc().unwrap().node_hash(),
        );

        self.tx_leader
            .send((leader.clone(), new_view))
            .await
            .map_err(|_| HotStuffError::SendError)?;
        // We store that we are running an election, so we know what to send when other nodes asks.
        self.election_in_progress.insert((shard, payload_id));
        // There is a lower timeout for local committee (delta).
        self.pacemaker
            .start_timer(
                PacemakerEvents::LocalCommittee(epoch, shard, payload_id),
                self.network_latency,
            )
            .await
            .unwrap();

        // We wait for the foreign committee twice as long (2*delta).
        // We start timer for each involved shard, the pacemaker will take care of the duplicates. Because if we are in
        // more than one committees this can be called multiple times. We have to take care of the local shards. They
        // are added separately above.
        for shard in involved_shards {
            if !local_shards.contains(&shard) {
                // This is shard for foreign committee.
                self.pacemaker
                    .start_timer(
                        PacemakerEvents::ForeignCommittee(epoch, shard, payload_id),
                        self.network_latency * 2,
                    )
                    .await
                    .unwrap();
            }
        }
        Ok(())
    }

    /// Step 2: The leader receives a NewView from all committee members. The payload and QC are persisted.
    ///         Once $n - f$ NewViews have been received, a Proposal is sent to the replicas, see
    ///         [HotstuffWaiter::on_beat].
    async fn leader_on_receive_new_view(
        &mut self,
        from: TAddr,
        shard: ShardId,
        qc: QuorumCertificate,
        payload: TPayload,
    ) -> Result<(), HotStuffError> {
        let payload_id = payload.to_id();
        info!(
            target: LOG_TARGET,
            "🔥 Receive NEWVIEW for payload {}, shard {} and height {}",
            payload_id,
            shard,
            qc.node_height()
        );

        let epoch = self.epoch_manager.current_epoch().await?;
        self.validate_from_committee(&from, epoch, shard).await?;
        // We dont expect signatures from other VNs at this point
        // TODO: should be 1, we expect the sender to sign their QC
        self.validate_qc(&qc, 0).await?;
        self.shard_store.with_write_tx(|tx| {
            self.update_high_qc(tx, from.clone(), qc)?;
            tx.save_payload(payload)?;
            Ok::<_, HotStuffError>(())
        })?;

        // Take note of unique NEWVIEWs so that we can count them
        let entry = self.newview_message_counts.entry((shard, payload_id)).or_default();
        entry.insert(from);
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    /// Step 4: Leader sends a Proposal to replica. A new leaf node is created that builds
    /// on the previous tree or else a genesis node is created and proposed.
    async fn leader_on_propose(&mut self, shard: ShardId, payload_id: PayloadId) -> Result<(), HotStuffError> {
        let high_qc;
        let payload;
        let current_leaf_node;
        {
            let tx = self.shard_store.create_read_tx()?;
            current_leaf_node = tx.get_leaf_node(&payload_id, &shard)?;
            high_qc = tx.get_high_qc_for(payload_id, shard)?;
            // The high QC could be from a previous payload, we want to propose for this payload
            payload = tx.get_payload(&payload_id)?;
        }

        let involved_shards = payload.involved_shards();
        let epoch = self.epoch_manager.current_epoch().await?;
        let members = self
            .epoch_manager
            .get_committees(epoch, &involved_shards)
            .await?
            .into_iter()
            .flat_map(|allocation| allocation.committee.members)
            .collect::<HashSet<_>>();

        // Create leaf node
        let leaf_node: HotStuffTreeNode<TAddr, TPayload>;
        {
            let mut tx = self.shard_store.create_write_tx()?;

            // TODO: We could only propose the pledge here and actually pledge it in on_receive_proposal
            let local_pledge = tx.pledge_object(shard, payload_id, NodeHeight(0))?;

            let (parent_hash, parent_height, parent_payload_height, maybe_payload) = if current_leaf_node.is_genesis() {
                (TreeNodeHash::zero(), NodeHeight(0), NodeHeight(0), Some(payload))
            } else {
                let node = tx.get_node(current_leaf_node.hash())?;
                (*node.hash(), node.height(), node.payload_height(), None)
            };

            leaf_node = HotStuffTreeNode::new(
                parent_hash,
                shard,
                parent_height + NodeHeight(1),
                payload_id,
                // We only need to send the payload for the genesis node, we choose not to include it to reduce
                // message size.
                maybe_payload,
                parent_payload_height + NodeHeight(1),
                *self.current_leader_round.entry((shard, payload_id)).or_default(),
                Some(local_pledge),
                epoch,
                self.public_key.clone(),
                high_qc,
            );

            info!(
                target: LOG_TARGET,
                "🌿 PROPOSING new leaf node {} {} in phase {:?} ({}) for payload {} shard {}",
                leaf_node.height(),
                leaf_node.hash(),
                leaf_node.payload_phase(),
                leaf_node.payload_height(),
                payload_id,
                shard,
            );
            tx.commit()?;
        }

        // send to all replicas for all shards, including ourselves
        self.tx_broadcast
            .send((
                HotStuffMessage::new_proposal(leaf_node, shard),
                members.into_iter().collect(),
            ))
            .await?;

        Ok(())
    }

    /// Step 5: A replica receives a Proposal from the leader. The replicas including the leader, validate the proposal
    /// and, once proposals for all shards have been received, send votes for all shards.
    #[allow(clippy::too_many_lines)]
    async fn on_receive_proposal(
        &mut self,
        from: TAddr,
        node: HotStuffTreeNode<TAddr, TPayload>,
    ) -> Result<(), HotStuffError> {
        let payload_id = node.payload_id();
        info!(
            target: LOG_TARGET,
            "🔥 Receive PROPOSAL for payload {}, shard {}, height {}, payload phase {:?}, hash {} from {}",
            payload_id,
            node.shard(),
            node.height(),
            node.payload_phase(),
            node.hash(),
            from,
        );

        self.validate_proposal(&node)?;

        // We remove the shard from pacemaker, so it will not trigger. We don't have to check if it's local shard or
        // not, we just remove them both, one of them will not exists, but pacemaker will take care of that.
        self.pacemaker
            .stop_timer(PacemakerEvents::LocalCommittee(node.epoch(), node.shard(), payload_id))
            .await?;

        self.pacemaker
            .stop_timer(PacemakerEvents::ForeignCommittee(
                node.epoch(),
                node.shard(),
                payload_id,
            ))
            .await?;

        // We did receive something from the leader, so he was elected.
        self.election_in_progress.remove(&(node.shard(), payload_id));

        let shard = node.shard();
        let payload;
        let last_vote_height;
        let last_leader_round;
        let locked_node;
        let locked_height;
        {
            let mut tx = self.shard_store.create_write_tx()?;

            (last_vote_height, last_leader_round) = tx.get_last_voted_height(node.shard(), node.payload_id())?;
            let (l_node, l_height) = tx.get_locked_node_hash_and_height(payload_id, shard)?;
            locked_node = l_node;
            locked_height = l_height;

            payload = if let Some(node_payload) = node.payload() {
                tx.save_payload(node_payload.clone())?;
                node_payload.clone()
            } else {
                tx.get_payload(&node.payload_id())?
            };
            tx.commit()?;
        }

        let involved_shards = payload.involved_shards();
        // If we have not previously voted on this payload and the node extends the current locked node, then we vote
        // Or if we already voted for this height but there was an election
        if (last_vote_height == NodeHeight(0) ||
            node.height() > last_vote_height ||
            (node.height() == last_vote_height && node.leader_round() > last_leader_round)) &&
            (*node.parent() == locked_node || node.height() > locked_height)
        {
            let proposed_nodes = self.shard_store.with_write_tx(|tx| {
                tx.save_node(node.clone())?;
                tx.save_leader_proposals(
                    node.shard(),
                    node.payload_id(),
                    node.payload_height(),
                    node.leader_round(),
                    node.clone(),
                )?;
                tx.get_leader_proposals(node.payload_id(), node.payload_height(), &involved_shards)
            })?;
            // We group proposal by the shard id.
            let mut proposed_nodes_grouped_by_shard_id: HashMap<ShardId, Vec<HotStuffTreeNode<TAddr, TPayload>>> =
                HashMap::new();
            for proposed_node in proposed_nodes {
                proposed_nodes_grouped_by_shard_id
                    .entry(proposed_node.shard())
                    .or_default()
                    .push(proposed_node);
            }
            // And now for each shard id we select only one proposal
            let mut proposed_nodes = Vec::new();
            for (_shard_id, nodes) in proposed_nodes_grouped_by_shard_id.drain() {
                proposed_nodes.push(nodes.into_iter().max_by_key(|node| node.leader_round()).unwrap());
            }

            // Check the number of leader proposals for <shard, payload, node height>
            // i.e. all proposed nodes for the shards for the payload are on the same hotstuff phase (payload height)
            if proposed_nodes.len() < involved_shards.len() {
                info!(
                    target: LOG_TARGET,
                    "🔥 Waiting for more leader proposals ({}/{}) before voting on payload {}, height {}",
                    proposed_nodes.len(),
                    involved_shards.len(),
                    payload_id,
                    node.payload_height()
                );

                self.update_nodes(&node)?;
                return Ok(());
            }

            match self.decide_and_vote_on_all_nodes(payload, proposed_nodes).await {
                Ok(_) => {},
                Err(err @ HotStuffError::AllShardsRejected { .. }) => {
                    self.publish_event(HotStuffEvent::Failed(payload_id, err.to_string()));
                },
                Err(err) => return Err(err),
            }

            let mut tx = self.shard_store.create_write_tx()?;
            tx.set_last_voted_height(node.shard(), node.payload_id(), node.height(), node.leader_round())?;
            tx.commit()?;
        } else {
            info!(
                target: LOG_TARGET,
                "🔥 Not ready to vote on payload {}, height {}, last_vote_height {}, locked_height {}",
                payload_id,
                node.height(),
                last_vote_height,
                locked_height
            );
        }
        self.update_nodes(&node)?;
        // If all pledges for all shards and complete, then we can persist the payload changes
        self.finalize_payload(&involved_shards, &node).await?;

        Ok(())
    }

    // Pacemaker triggered for the local leader. Replica sends a newview.
    async fn pacemaker_trigger_local_leader_fail(
        &mut self,
        epoch: Epoch,
        payload_id: PayloadId,
        shard_id: ShardId,
    ) -> Result<(), HotStuffError> {
        let high_qc = self
            .shard_store
            .create_read_tx()?
            .get_high_qc_for(payload_id, shard_id)
            .optional()?
            .unwrap_or_else(|| QuorumCertificate::genesis(epoch, payload_id, shard_id));

        // TODO: should we send this? the new leader probably has it
        let payload = self.shard_store.create_read_tx()?.get_payload(&payload_id).unwrap();

        // We leave the payload empty, this is just new round, so the nodes should have it.
        let new_view = HotStuffMessage::new_view(high_qc, shard_id, payload);
        let current_leader = self
            .current_leader_round
            .entry((shard_id, payload_id))
            .and_modify(|round| *round += 1)
            .or_default();
        let committee = self.epoch_manager.get_committee(epoch, shard_id).await?;
        let leader = self
            .leader_strategy
            .get_leader(&committee, payload_id, shard_id, *current_leader);
        self.tx_leader
            .send((leader.clone(), new_view))
            .await
            .map_err(|_| HotStuffError::SendError)?;
        self.pacemaker
            .start_timer(
                PacemakerEvents::LocalCommittee(epoch, shard_id, payload_id),
                self.network_latency * (2 << *current_leader),
            )
            .await
            .unwrap();
        // Election started
        self.election_in_progress.insert((shard_id, payload_id));
        Ok(())
    }

    async fn pacemaker_trigger_foreign_leader_fail(
        &self,
        epoch: Epoch,
        payload_id: PayloadId,
        shard_id: ShardId,
    ) -> Result<(), HotStuffError> {
        // TODO: If the pacemaker is triggered a second time for the same (epoch, shard_id, payload_id),
        // means that the the second message request (on f + 1 node messages, from the foreign committee)
        // didn't receive a response. This is possibly due to a lack of local consensus on the foreign committee
        // (e.g., malformed transaction, etc). In this case, the honest 2f + 1 honest replicas in the foreign
        // committee will reject the Payload and broadcast their decision to all involved shards, including ours.
        // Therefore, on the call of `on_receive_proposal`, we will also reject this Payload. Summarizing, we
        // shouldn't need to add further logic in the current method to deal with a second pacemaker trigger.

        // The foreign leader failed to send a proposal. Let's check what's going on in the
        // foreign committee. We will asks f+1 nodes, so that at least one of the should be honest, so if the
        // committee reached a decision, it will sent it back. Now broadcast to these nodes that I didn't
        // received new proposal from their leader, someone should have it, or the QC was not reached. Send the
        // last know height (0 in case we haven't received any proposal yet from this committee).
        let foreign_committee = self.epoch_manager.get_committee(epoch, shard_id).await.unwrap();
        let how_many_to_ask = foreign_committee.len() / 3 + 1; // f+1

        // We select the subset of the committee by random.
        let selected_foreing_committee_members: Vec<_> = foreign_committee
            .members
            .choose_multiple(&mut rand::thread_rng(), how_many_to_ask)
            .cloned()
            .collect();
        let last_height = self
            .shard_store
            .create_read_tx()?
            .get_last_payload_height_for_leader_proposal(payload_id, shard_id)?;
        self.tx_recovery_broadcast
            .send((
                RecoveryMessage::MissingProposal(epoch, shard_id, payload_id, last_height),
                selected_foreing_committee_members,
            ))
            .await
            .unwrap();
        self.pacemaker
            .start_timer(
                PacemakerEvents::ForeignCommittee(epoch, shard_id, payload_id),
                self.network_latency * 2,
            )
            .await
            .unwrap();
        Ok(())
    }

    // Sender asks if there is a newer local state, that he is not in possession of. This means that Sender didn't
    // receive a response from our local leader. In this case, we might be in the presence of a faulty leader
    async fn on_receive_missing_proposal(
        &self,
        from: TAddr,
        epoch: Epoch,
        shard_id: ShardId,
        payload_id: PayloadId,
        last_height: NodeHeight,
    ) -> Result<(), HotStuffError> {
        // If the current replica has not stored the payload, we simply don't do anything else and return
        let payload = match self.shard_store.create_read_tx()?.get_payload(&payload_id).optional()? {
            Some(payload) => payload,
            None => {
                info!(
                    target: LOG_TARGET,
                    "Payload = {} is missing from node shard store", payload_id
                );
                return Ok(());
            },
        };

        let involved_shards = payload.involved_shards();
        let local_shards = self
            .epoch_manager
            .filter_to_local_shards(epoch, &from, involved_shards.as_ref())
            .await?;

        // if `from` doesn't belong to any shard involved in processing the current payload, we should ignore it
        if local_shards.is_empty() {
            info!(
                target: LOG_TARGET,
                "Missing proposal sent by node = {} which is not processing current payload = {}", from, payload_id
            );
            return Ok(());
        }

        let leader_proposals =
            self.shard_store
                .create_write_tx()?
                .get_leader_proposals(payload_id, last_height + NodeHeight(1), &[shard_id]);
        // We know the last payload, so we check if we have a higher height
        if let Ok(result) = leader_proposals {
            assert!(
                result.len() <= 1,
                "We can't have more than one proposal at certain height for one particular shard"
            );
            let committee = self.epoch_manager.get_committee(epoch, shard_id).await?;
            if !committee.contains(&self.public_key) {
                info!(
                    target: LOG_TARGET,
                    "Current node with public key = {} is not involved in processing payload = {}",
                    self.public_key,
                    payload_id
                );
                return Err(HotStuffError::NodeIsNotInvolvedInPayload { payload_id });
            }
            // Do we have such a node? If not, maybe our leader failed. So I don't send anything and this node will
            // receive its own pacemaker trigger as well.
            if let Some(node) = result.get(0) {
                // We use the leader channel, but for the node it doesn't really matter, because it doesn't check where
                // is the node coming from as long it's valid.
                self.tx_leader
                    .send((from, HotStuffMessage::new_proposal(node.clone(), shard_id)))
                    .await?;
            } else {
                // We check if we are electing a new leader, right now.
                if self.election_in_progress.contains(&(shard_id, payload_id)) {
                    self.tx_recovery
                        .send((RecoveryMessage::ElectionInProgress(epoch, shard_id, payload_id), from))
                        .await?;
                }
                // If we are not electing a new leader, then we don't send anything, that means the TX is invalid. This
                // will also trigger invalid in all committees that are asking us for update.
            }
        }
        Ok(())
    }

    // We are running a pacemaker for the proposal. But the other committee is still having an election. Let's postpone
    // the pacemaker.
    async fn on_receive_election_in_progress(
        &self,
        _from: TAddr,
        epoch: Epoch,
        shard_id: ShardId,
        payload_id: PayloadId,
    ) -> Result<(), HotStuffError> {
        // TODO: if there is a byzantine replica, he can always send election in progress. We should count the election
        // rounds. Be aware that the election rounds are based on delta time, and this trigger is 2*delta.
        // We restart the timer.
        self.pacemaker
            .stop_timer(PacemakerEvents::ForeignCommittee(epoch, shard_id, payload_id))
            .await?;
        self.pacemaker
            .start_timer(
                PacemakerEvents::ForeignCommittee(epoch, shard_id, payload_id),
                self.network_latency * 2,
            )
            .await?;
        Ok(())
    }

    async fn on_receive_recovery_message(&self, from: TAddr, msg: RecoveryMessage) -> Result<(), HotStuffError> {
        match msg {
            RecoveryMessage::MissingProposal(epoch, shard_id, payload_id, last_height) => {
                self.on_receive_missing_proposal(from, epoch, shard_id, payload_id, last_height)
                    .await
            },
            RecoveryMessage::ElectionInProgress(epoch, shard_id, payload_id) => {
                self.on_receive_election_in_progress(from, epoch, shard_id, payload_id)
                    .await
            },
        }
    }

    async fn on_pacemaker_trigger(&mut self, message: PacemakerEvents) -> Result<(), HotStuffError> {
        match message {
            PacemakerEvents::LocalCommittee(epoch, shard_id, payload_id) => {
                self.pacemaker_trigger_local_leader_fail(epoch, payload_id, shard_id)
                    .await
            },
            PacemakerEvents::ForeignCommittee(epoch, shard_id, payload_id) => {
                self.pacemaker_trigger_foreign_leader_fail(epoch, payload_id, shard_id)
                    .await
            },
        }
    }

    /// Checks that all pledges have been resolved (completed/abandoned). If so, atomically commit the changeset for the
    /// local shards
    async fn finalize_payload(
        &self,
        involved_shards: &[ShardId],
        node: &HotStuffTreeNode<TAddr, TPayload>,
    ) -> Result<(), HotStuffError> {
        // TODO(perf): Perhaps mark local pledges as local and use their shard ids
        let local_shards = self
            .epoch_manager
            .filter_to_local_shards(node.epoch(), &self.public_key, involved_shards)
            .await?;

        let mut tx = self.shard_store.create_write_tx()?;
        // TODO(perf): can count completed/abandoned pledges and only load if necessary
        let resolved_pledges = tx.get_resolved_pledges_for_payload(node.payload_id())?;
        assert!(
            resolved_pledges.len() <= involved_shards.len(),
            "More pledges than involved shards"
        );
        // Check if we have resolved all pledges, if so, we are ready to commit resultant substate changes
        if resolved_pledges.len() == involved_shards.len() {
            let payload_result = tx.get_payload_result(&node.payload_id())?;
            match &payload_result.finalize_result.result {
                TransactionResult::Accept(diff) => {
                    if resolved_pledges
                        .iter()
                        .any(|pledge| pledge.abandoned_by_tree_node_hash.is_some())
                    {
                        // Fail immediately
                        self.publish_event(HotStuffEvent::Failed(
                            node.payload_id(),
                            "Payload was accepted by this node but some pledges were abandoned".to_string(),
                        ));
                        return Ok(());
                    }

                    let local_change_set = extract_changes_for_shards(&local_shards, node.payload_id(), diff)?;
                    for pledge in resolved_pledges {
                        // Only persist local shards
                        if let Some(changes) = local_change_set.get(&pledge.shard_id) {
                            let node_hash = pledge
                                .completed_by_tree_node_hash
                                .expect("[finalize_payload] Pledge MUST be completed");
                            let node = tx.get_node(&node_hash)?;
                            tx.save_substate_changes(node, changes)?;
                        }
                    }

                    self.publish_event(HotStuffEvent::OnFinalized(
                        Box::new(node.justify().clone()),
                        payload_result.finalize_result,
                    ));
                },
                TransactionResult::Reject(reason) => {
                    self.publish_event(HotStuffEvent::Failed(node.payload_id(), reason.to_string()));
                },
            }
        }

        tx.commit()?;

        Ok(())
    }

    fn validate_proposal(&self, node: &HotStuffTreeNode<TAddr, TPayload>) -> Result<(), ProposalValidationError> {
        let payload_height = node.justify().payload_height() + NodeHeight(1);
        if !node.is_genesis() && node.payload_height() != payload_height {
            return Err(ProposalValidationError::NodePayloadHeightIncorrect {
                node_payload_height: node.payload_height(),
                justify_payload_height: payload_height,
            });
        }

        if node.payload_id() != node.justify().payload_id() {
            return Err(ProposalValidationError::NodePayloadDoesNotMatchJustifyPayload {
                node_payload: node.payload_id(),
                justify_payload: node.justify().payload_id(),
            });
        }

        if node.shard() != node.justify().shard() {
            return Err(ProposalValidationError::NodeShardDoesNotMatchJustifyPayload {
                node_shard: node.shard(),
                justify_shard: node.justify().shard(),
            });
        }

        let max_node_height = self.consensus_constants.max_payload_height();
        if node.payload_height() > max_node_height {
            return Err(ProposalValidationError::PayloadHeightIsTooHigh {
                actual: node.payload_height(),
                max: max_node_height,
            });
        }

        match node.local_pledge() {
            Some(pledge) => {
                if pledge.pledged_to_payload != node.payload_id() {
                    return Err(ProposalValidationError::PledgePayloadMismatch {
                        shard: node.shard(),
                        pledged_payload: pledge.pledged_to_payload,
                    });
                }
            },

            None => return Err(ProposalValidationError::LocalPledgeIsNone),
        }
        // if node.payload_height() > NodeHeight(0) && node.justify().decision() != &QuorumDecision::Accept {
        //     return Err(HotStuffError::JustifyIsNotAccepted);
        // }

        Ok(())
    }

    async fn decide_and_vote_on_all_nodes(
        &self,
        payload: TPayload,
        proposed_nodes: Vec<HotStuffTreeNode<TAddr, TPayload>>,
    ) -> Result<(), HotStuffError> {
        let payload_id = payload.to_id();
        let involved_shards = payload.involved_shards();

        // Find the shards that this node is responsible for out of all the shards involved
        // TODO: Perhaps we can determine the epoch for the payload and then use it. This assumes that all nodes have
        //       the same epoch as the first node (should we validate that this is the case?)
        let first_node = proposed_nodes
            .get(0)
            .expect("invariant failed: decide_and_vote_on_all_nodes called with empty nodes");
        let epoch = first_node.epoch();
        let local_shards = self
            .epoch_manager
            .filter_to_local_shards(epoch, &self.public_key, &involved_shards)
            .await?;
        let vn_shard_key = self
            .epoch_manager
            .get_validator_shard_key(epoch, self.public_key.clone())
            .await?;
        let vn_mmr = self.epoch_manager.get_validator_node_mmr(epoch).await?;

        let shard_pledges = proposed_nodes
            .iter()
            .map(|proposed_node| ShardPledge {
                shard_id: proposed_node.shard(),
                node_hash: *proposed_node.hash(),
                pledge: proposed_node
                    .local_pledge()
                    .expect("Pledge is empty. This should have been checked previously")
                    .clone(),
            })
            .collect::<ShardPledgeCollection>();

        // Find all rejected nodes, if any are rejected then we vote to reject all our local shards
        let is_all_rejected =
            self.check_for_other_shard_rejections(&payload_id, &proposed_nodes, shard_pledges.pledge_hash())?;

        for node in proposed_nodes {
            // Check that this node is a node we need to vote on
            if !local_shards.contains(&node.shard()) {
                if node.payload_phase() != HotstuffPhase::Decide &&
                    !(is_all_rejected && node.payload_phase() == HotstuffPhase::PreCommit)
                {
                    // If we are not in decide phase, we are going to send a vote, we expect that the vote will be
                    // propagated to foreign committees and that we will get a proposal from respective foreing leaders.
                    self.pacemaker
                        .start_timer(
                            PacemakerEvents::ForeignCommittee(epoch, node.shard(), payload_id),
                            self.network_latency * 2,
                        )
                        .await?;
                }
                continue;
            }

            // In decide phase we don't send a vote
            if node.payload_phase() == HotstuffPhase::Decide {
                info!(
                    target: LOG_TARGET,
                    "🔥 Decided on node {} for payload {}, shard {}",
                    node.hash(),
                    node.payload_id(),
                    node.shard()
                );
                continue;
            }

            // If all proposals are rejections and we have proof that all validators have voted in this way,
            // we've sent our last REJECT vote in the PREPARE round so we dont vote again.
            if is_all_rejected && node.payload_phase() == HotstuffPhase::PreCommit {
                // Abandon early because we are not continuing to vote so will never reach the DECIDE for the chain
                self.shard_store.with_write_tx(|tx| {
                    tx.abandon_pledges(node.shard(), node.payload_id(), node.hash())
                        // If the substate was pledged to a different payload, we didn't pledge for this payload so the pledge may not exist
                        .optional()
                })?;
                info!(
                    target: LOG_TARGET,
                    "🔥 Skipping PRECOMMIT REJECT vote on node {} for payload {}, shard {}",
                    node.hash(),
                    node.payload_id(),
                    node.shard()
                );

                continue;
            }

            let finalize_result = self.decide(&node, payload.clone(), &shard_pledges)?;

            let vote_msg = self.create_vote(
                *node.hash(),
                shard_pledges.clone(),
                &finalize_result.result,
                vn_shard_key,
                &vn_mmr,
            )?;

            let leader = self.get_leader(&node).await?;
            self.tx_vote_message.send((vote_msg, leader)).await?;
            // We add timer for each local committee.
            self.pacemaker
                .start_timer(
                    PacemakerEvents::LocalCommittee(epoch, node.shard(), payload_id),
                    self.network_latency,
                )
                .await?;
        }

        if is_all_rejected {
            let payload_result = self.shard_store.with_read_tx(|tx| tx.get_payload_result(&payload_id))?;
            return Err(HotStuffError::AllShardsRejected {
                payload_id,
                reason: payload_result
                    .finalize_result
                    .result
                    .reject()
                    .map(|r| r.to_string())
                    .unwrap_or_else(|| "Unknown reason".to_string()),
            });
        }

        Ok(())
    }

    /// Checks for other shard rejections, if at least one is encountered and we were voting ACCEPT, we change our
    /// payload result to reject. We return true if all proposals are rejections, otherwise false
    fn check_for_other_shard_rejections(
        &self,
        payload_id: &PayloadId,
        proposed_nodes: &[HotStuffTreeNode<TAddr, TPayload>],
        pledge_hash: FixedHash,
    ) -> Result<bool, HotStuffError> {
        let rejected_nodes = proposed_nodes
            .iter()
            .filter(|n| n.justify().decision().is_reject())
            .collect::<Vec<_>>();

        if !rejected_nodes.is_empty() {
            let mut tx = self.shard_store.create_write_tx()?;
            let current_payload_result = tx.get_payload_result(payload_id)?;
            // Only change to reject is we arent already rejecting for another reason
            if current_payload_result.finalize_result.is_accept() {
                // If a shard has been rejected, we vote to reject all our shards
                let finalize_result = FinalizeResult::reject(
                    payload_id.into_array().into(),
                    RejectReason::ShardRejected(
                        rejected_nodes
                            .iter()
                            .map(|n| format!("{}({:?})", n.shard(), n.justify().decision()))
                            .collect::<Vec<_>>()
                            .join(", "),
                    ),
                );

                info!(
                    target: LOG_TARGET,
                    "🔥 {} rejected shard(s) for payload {}. Voting to REJECT all local shards.",
                    rejected_nodes.len(),
                    payload_id
                );

                tx.update_payload_result(payload_id, PayloadResult {
                    finalize_result,
                    pledge_hash,
                })?;
            }
            tx.commit()?;
        }

        Ok(rejected_nodes.len() == proposed_nodes.len())
    }

    #[allow(clippy::too_many_lines)]
    fn decide(
        &self,
        node: &HotStuffTreeNode<TAddr, TPayload>,
        payload: TPayload,
        shard_pledges: &ShardPledgeCollection,
    ) -> Result<FinalizeResult, HotStuffError> {
        let pledge_hash = shard_pledges.pledge_hash();
        // On every phase, validate that the pledges are pledged to this payload.
        for pledge in shard_pledges.iter() {
            if pledge.pledge.pledged_to_payload != node.payload_id() {
                let finalize_result = FinalizeResult::reject(
                    payload.to_id().into_array().into(),
                    RejectReason::ShardPledgedToAnotherPayload(
                        HotStuffError::ShardPledgedToDifferentPayload {
                            shard: pledge.shard_id,
                            pledged_payload: pledge.pledge.pledged_to_payload,
                            expected: node.payload_id(),
                        }
                        .to_string(),
                    ),
                );

                self.shard_store.with_write_tx(|tx| {
                    tx.update_payload_result(&node.payload_id(), PayloadResult {
                        finalize_result: finalize_result.clone(),
                        pledge_hash,
                    })
                })?;

                return Ok(finalize_result);
            }
        }

        match node.payload_phase() {
            HotstuffPhase::Prepare => {
                let payload_id = payload.to_id();
                info!(
                    target: LOG_TARGET,
                    "🔥 Executing payload in PREPARE phase: {}", payload_id
                );

                let pledge = self
                    .shard_store
                    .with_write_tx(|tx| tx.pledge_object(node.shard(), node.payload_id(), node.payload_height()))?;

                // If an active pledge already exists for another payload, we REJECT this payload.
                if pledge.pledged_to_payload != node.payload_id() {
                    let finalize_result = FinalizeResult::reject(
                        node.payload_id().into_array().into(),
                        RejectReason::ShardPledgedToAnotherPayload(format!(
                            "Shard {} is pledged to another payload {}",
                            node.shard(),
                            pledge.pledged_to_payload
                        )),
                    );
                    self.shard_store.with_write_tx(|tx| {
                        tx.update_payload_result(&node.payload_id(), PayloadResult {
                            finalize_result: finalize_result.clone(),
                            pledge_hash,
                        })
                    })?;

                    return Ok(finalize_result);
                }
                let finalize_result = match task::block_in_place(|| self.execute(payload, shard_pledges, node.epoch()))
                {
                    Ok(finalize_result) => finalize_result,
                    Err(err) => FinalizeResult::reject(
                        payload_id.into_array().into(),
                        RejectReason::ExecutionFailure(err.to_string()),
                    ),
                };

                if let TransactionResult::Accept(ref diff) = finalize_result.result {
                    match Self::validate_pledges(shard_pledges, diff) {
                        Ok(_) => {
                            self.shard_store.with_write_tx(|tx| {
                                tx.update_payload_result(&node.payload_id(), PayloadResult {
                                    finalize_result: finalize_result.clone(),
                                    pledge_hash,
                                })
                            })?;
                            Ok(finalize_result)
                        },
                        Err(e @ HotStuffError::MissingPledges(_)) => {
                            let finalize_result = FinalizeResult::reject(
                                payload_id.into_array().into(),
                                RejectReason::ShardsNotPledged(e.to_string()),
                            );
                            self.shard_store.with_write_tx(|tx| {
                                tx.update_payload_result(&node.payload_id(), PayloadResult {
                                    finalize_result: finalize_result.clone(),
                                    pledge_hash,
                                })
                            })?;

                            Ok(finalize_result)
                        },
                        Err(e) => Err(e),
                    }
                } else {
                    self.shard_store.with_write_tx(|tx| {
                        tx.update_payload_result(&node.payload_id(), PayloadResult {
                            finalize_result: finalize_result.clone(),
                            pledge_hash,
                        })
                    })?;

                    Ok(finalize_result)
                }
            },
            _phase => {
                let finalize_result = self
                    .shard_store
                    .with_read_tx(|tx| tx.get_payload_result(&node.payload_id()))?;

                if pledge_hash != finalize_result.pledge_hash {
                    return Err(HotStuffError::ShardPledgesChanged {
                        payload_id: node.payload_id(),
                    });
                }

                Ok(finalize_result.finalize_result)
            },
        }
    }

    /// Checks that all shards have been pledged correctly, if not, will return the list of shards that
    /// were not pledged
    fn validate_pledges(shard_pledges: &[ShardPledge], diff: &SubstateDiff) -> Result<(), HotStuffError> {
        let mut missing_pledges = vec![];

        // If we've downed the substate, the pledges should be up
        for (address, version) in diff.down_iter() {
            let shard_id = ShardId::from_address(address, *version);
            match shard_pledges.iter().find(|p| p.pledge.shard_id == shard_id) {
                Some(ShardPledge {
                    pledge: ObjectPledge { current_state, .. },
                    ..
                }) => {
                    // To down a substate it should be pledged as up
                    if !matches!(current_state, SubstateState::Up { .. }) {
                        missing_pledges.push((shard_id, SubstateChange::Exists, address.clone(), *version));
                    }
                },
                None => missing_pledges.push((shard_id, SubstateChange::Exists, address.clone(), *version)),
            }
        }

        for (addr, substate) in diff.up_iter() {
            let shard_id = ShardId::from_address(addr, substate.version());
            match shard_pledges.iter().find(|p| p.pledge.shard_id == shard_id) {
                Some(ShardPledge {
                    pledge:
                        ObjectPledge {
                            current_state,
                            pledged_to_payload,
                            ..
                        },
                    ..
                }) => {
                    // To up a substate it should be pledged as never existing
                    match current_state {
                        SubstateState::DoesNotExist => {},
                        SubstateState::Up { created_by, .. } => {
                            return Err(HotStuffError::InvalidPledge {
                                shard: shard_id,
                                pledged_payload: *pledged_to_payload,
                                details: format!("Pledged substate is already UP'd by payload {}", created_by),
                            });
                        },
                        SubstateState::Down { deleted_by } => {
                            return Err(HotStuffError::InvalidPledge {
                                shard: shard_id,
                                pledged_payload: *pledged_to_payload,
                                details: format!("Pledged substate is already DOWN'd by payload {}", deleted_by),
                            });
                        },
                    }
                },
                None => missing_pledges.push((shard_id, SubstateChange::Create, addr.clone(), substate.version())),
            }
        }

        if missing_pledges.is_empty() {
            Ok(())
        } else {
            // Sort them so that they are the same for all VNs.
            missing_pledges.sort_by(|a, b| a.0.cmp(&b.0));
            Err(HotStuffError::MissingPledges(missing_pledges))
        }
    }

    /// Step 6: The leader receives votes from the local shard, and once it has enough ($n - f$) votes, it commits a
    /// high QC and sends the next round of proposals.
    async fn leader_on_receive_vote(&mut self, from: TAddr, msg: VoteMessage) -> Result<(), HotStuffError> {
        info!(
            target: LOG_TARGET,
            "🔥 Receive {:?} VOTE for node {} from {}",
            msg.decision(),
            msg.local_node_hash(),
            from,
        );

        let mut on_propose = None;
        let node;
        {
            let tx = self.shard_store.create_read_tx()?;
            // Avoid duplicates
            if tx.has_vote_for(&from, msg.local_node_hash())? {
                println!("🔥 Vote with node hash {} already received", msg.local_node_hash());
                return Ok(());
            }

            node = tx
                .get_node(&msg.local_node_hash())
                .optional()?
                .ok_or(HotStuffError::InvalidVote(format!(
                    "Node with hash {} not found",
                    msg.local_node_hash()
                )))?;
            if *node.proposed_by() != self.public_key {
                return Err(HotStuffError::NotTheLeader);
            }
        }

        let valid_committee = self.epoch_manager.get_committee(node.epoch(), node.shard()).await?;
        {
            if !valid_committee.contains(&from) {
                return Err(HotStuffError::ReceivedMessageFromNonCommitteeMember);
            }
            let mut tx = self.shard_store.create_write_tx()?;

            // Collect votes
            tx.save_received_vote_for(from, msg.local_node_hash(), msg.clone())?;

            let votes = tx.get_received_votes_for(msg.local_node_hash())?;

            if votes.len() == valid_committee.consensus_threshold() {
                let validator_metadata = votes.iter().map(|v| v.validator_metadata().clone()).collect();

                // TODO: Check all votes
                let main_vote = votes.get(0).unwrap();

                let qc = QuorumCertificate::new(
                    node.payload_id(),
                    node.payload_height(),
                    *node.hash(),
                    node.height(),
                    node.shard(),
                    node.epoch(),
                    main_vote.decision(),
                    main_vote.all_shard_pledges().clone(),
                    validator_metadata,
                );
                self.update_high_qc(&mut tx, node.proposed_by().clone(), qc)?;

                on_propose = Some((node.shard(), node.payload_id()));
            }

            // commit the transaction
            tx.commit()?;
        }

        // Propose the next node
        if let Some((shard_id, payload_id)) = on_propose {
            // TODO: This should go in a some component that controls message flows and events
            let epoch = self.epoch_manager.current_epoch().await?;
            let committee = self.epoch_manager.get_committee(epoch, shard_id).await?;
            if committee.is_empty() {
                return Err(HotStuffError::NoCommitteeForShard { shard: shard_id, epoch });
            }
            if self.is_leader(payload_id, shard_id, &committee)? {
                self.leader_on_propose(shard_id, payload_id).await?;
            }
        }
        Ok(())
    }

    fn get_newview_count_for(&self, shard: ShardId, payload_id: PayloadId) -> usize {
        self.newview_message_counts
            .get(&(shard, payload_id))
            .map(|unique_senders| unique_senders.len())
            .unwrap_or(0)
    }

    fn is_leader(
        &self,
        payload: PayloadId,
        shard: ShardId,
        committee: &Committee<TAddr>,
    ) -> Result<bool, HotStuffError> {
        // TODO: What if the leader doesn't know that he is the leader? e.g. didn't get the message. The leader (index
        // 0) failed. Now index 1 should be leader, but he doesn't know, so he ignores the newviews (should we ignore
        // the newviews?)
        Ok(self.leader_strategy.is_leader(
            &self.public_key,
            committee,
            payload,
            shard,
            *self.current_leader_round.get(&(shard, payload)).unwrap_or(&0),
        ))
    }

    async fn validate_from_committee(&self, from: &TAddr, epoch: Epoch, shard: ShardId) -> Result<(), HotStuffError> {
        if self.epoch_manager.get_committee(epoch, shard).await?.contains(from) {
            Ok(())
        } else {
            Err(HotStuffError::ReceivedMessageFromNonCommitteeMember)
        }
    }

    async fn validate_qc(&self, qc: &QuorumCertificate, min_signers: usize) -> Result<(), HotStuffError> {
        // extract all the pairs of signer-signature present in the QC
        let signer_signatures = Self::extract_signer_signatures_from_qc(qc);

        // the QC should not have repeated signers
        let signers_set = signer_signatures
            .iter()
            .map(|s| NodeAddressable::as_bytes(&s.0))
            .collect::<HashSet<_>>();
        if signer_signatures.len() != signers_set.len() {
            return Err(HotStuffError::InvalidQuorumCertificate(
                "duplicated signers".to_string(),
            ));
        }

        // check that the minimum quorum has been reached
        if signers_set.len() < min_signers {
            return Err(HotStuffError::InvalidQuorumCertificate(
                "insufficient quorum".to_string(),
            ));
        }

        // all merkle proofs for the signers must be valid
        let validator_node_root = self.epoch_manager.get_validator_node_merkle_root(qc.epoch()).await?;
        // TODO: Combine all validator merkle proofs before sending them
        for md in qc.validators_metadata() {
            md.merkle_proof
                .verify_leaf::<ValidatorNodeMmrHasherBlake256>(
                    &validator_node_root,
                    &*md.get_node_hash(),
                    LeafIndex(md.merkle_leaf_index as usize),
                )
                .map_err(|e| HotStuffError::InvalidQuorumCertificate(format!("invalid merkle proof: {}", e)))?;
        }

        // all signers must be included in the epoch committee for the shard
        let committee = self.epoch_manager.get_committee(qc.epoch(), qc.shard()).await?;
        let committee_set = committee.members.iter().map(|m| m.as_bytes()).collect::<HashSet<_>>();
        let all_signers_are_in_committee = signers_set.iter().all(|s| committee_set.contains(s));
        if !all_signers_are_in_committee {
            return Err(HotStuffError::InvalidQuorumCertificate(
                "some signers are not in committee".to_string(),
            ));
        }

        // all signatures must be valid
        let all_signatures_are_valid = signer_signatures
            .iter()
            .all(|(public_key, signature)| self.validate_vote(qc, public_key, signature));
        if !all_signatures_are_valid {
            return Err(HotStuffError::InvalidQuorumCertificate("invalid signature".to_string()));
        }

        Ok(())
    }

    fn validate_vote(&self, qc: &QuorumCertificate, public_key: &PublicKey, signature: &Signature) -> bool {
        let vote = VoteMessage::new(qc.node_hash(), *qc.decision(), qc.all_shard_pledges().clone());
        let challenge = vote.construct_challenge();
        self.signing_service
            .verify_for_public_key(public_key, signature, &*challenge)
    }

    fn extract_signer_signatures_from_qc(qc: &QuorumCertificate) -> Vec<(PublicKey, Signature)> {
        qc.validators_metadata()
            .iter()
            .map(|md| (md.public_key.clone(), md.signature.clone()))
            .collect()
    }

    /// See section 6, algorithm 4 in https://arxiv.org/pdf/1803.05069.pdf
    fn update_nodes(&self, node: &HotStuffTreeNode<TAddr, TPayload>) -> Result<(), HotStuffError> {
        let mut tx = self.shard_store.create_write_tx()?;
        // commit_node is at PRE-COMMIT phase
        self.update_high_qc(&mut tx, node.proposed_by().clone(), node.justify().clone())?;

        // b'' <- b*.justify.node
        let commit_node = match tx.get_node(&node.justify().node_hash()).optional()? {
            Some(node) => node,
            None => {
                tx.commit()?;
                return Ok(());
            },
        };

        // b' <- b''.justify.node
        let precommit_node = match tx.get_node(&commit_node.justify().node_hash()).optional()? {
            Some(node) => node,
            None => {
                tx.commit()?;
                return Ok(());
            },
        };

        let (_node_lock_hash, locked_node_height) =
            tx.get_locked_node_hash_and_height(node.payload_id(), node.shard())?;
        if precommit_node.height() > locked_node_height {
            info!(target: LOG_TARGET, "Updating locked node to: {}", precommit_node.hash());
            // precommit_node is at COMMIT phase
            tx.set_locked(
                precommit_node.payload_id(),
                precommit_node.shard(),
                *precommit_node.hash(),
                precommit_node.height(),
            )?;
        }

        // b <- b'.justify.node
        let prepare_node = precommit_node.justify().node_hash();
        if commit_node.parent() == precommit_node.hash() && *precommit_node.parent() == prepare_node {
            info!(
                target: LOG_TARGET,
                "✅ Node {} forms a 3-chain b'' = {}, b' = {}, b = {}",
                node.hash(),
                commit_node.hash(),
                precommit_node.hash(),
                prepare_node,
            );

            self.on_commit(&mut tx, node)?;
            tx.set_last_executed_height(node.shard(), node.payload_id(), node.height())?;
        } else {
            debug!(
                target: LOG_TARGET,
                "Node DOES NOT form a 3-chain b'' = {}, b' = {}, b = {}, b* = {}",
                commit_node.hash(),
                precommit_node.hash(),
                prepare_node,
                node.hash()
            );
        }

        tx.commit()?;

        Ok(())
    }

    fn update_high_qc(
        &self,
        tx: &mut TShardStore::WriteTransaction<'_>,
        proposed_by: TAddr,
        qc: QuorumCertificate,
    ) -> Result<(), HotStuffError> {
        let high_qc_height = tx
            .get_high_qc_for(qc.payload_id(), qc.shard())
            .optional()?
            .map(|hqc| hqc.node_height());

        if high_qc_height.map(|height| qc.node_height() > height).unwrap_or(true) {
            info!(
                target: LOG_TARGET,
                "🔥 UPDATE_HIGH_QC (node: {} {}, shard: {}, payload: {}, previous: {})",
                qc.node_height(),
                qc.node_hash(),
                qc.shard(),
                qc.payload_id(),
                high_qc_height.unwrap_or(NodeHeight(0)),
            );
            tx.set_leaf_node(
                qc.payload_id(),
                qc.shard(),
                qc.node_hash(),
                qc.payload_height(),
                qc.node_height(),
            )?;
            tx.insert_high_qc(proposed_by, qc.shard(), qc)?;
        }

        Ok(())
    }

    /// Commits the changeset and node including all parent nodes if not already done so.
    fn on_commit(
        &self,
        tx: &mut TShardStore::WriteTransaction<'_>,
        node: &HotStuffTreeNode<TAddr, TPayload>,
    ) -> Result<(), HotStuffError> {
        let last_exec_height = tx.get_last_executed_height(node.shard(), node.payload_id())?;
        if last_exec_height < node.height() {
            match node.payload_phase() {
                HotstuffPhase::Decide => {
                    info!(
                        target: LOG_TARGET,
                        "🔥 [on_commit] Committing payload {} in DECIDE phase",
                        node.payload_id()
                    );
                    let payload_result = tx.get_payload_result(&node.payload_id())?;
                    match payload_result.finalize_result.result {
                        TransactionResult::Accept(_) => {
                            tx.complete_pledges(node.shard(), node.payload_id(), node.hash())?;
                        },
                        TransactionResult::Reject(_) => {
                            info!(
                                target: LOG_TARGET,
                                "🔥 on_commit ABANDON pledge for payload {}, shard{}",
                                node.payload_id(),
                                node.shard()
                            );
                            tx.abandon_pledges(node.shard(), node.payload_id(), node.hash())
                                // With conflicting multi-shard payloads A and B, it may be that some pledges are for payload A and some for payload B.
                                // This results in both payloads being rejected, but also means we cannot count on the pledge existing for this node.
                                .optional()?;
                        },
                    }
                },
                phase => {
                    info!(
                        target: LOG_TARGET,
                        "🔥 [on_commit] node {} {} for payload {} in {:?} phase",
                        node.height(),
                        node.hash(),
                        node.payload_id(),
                        phase
                    );
                },
            }
        }

        Ok(())
    }

    fn execute(
        &self,
        payload: TPayload,
        shard_pledges: &ShardPledgeCollection,
        epoch: Epoch,
    ) -> Result<FinalizeResult, HotStuffError> {
        let maybe_payload_result = self
            .shard_store
            .with_read_tx(|tx| tx.get_payload_result(&payload.to_id()).optional())?;

        if let Some(payload_result) = maybe_payload_result {
            if shard_pledges.pledge_hash() == payload_result.pledge_hash {
                return Ok(payload_result.finalize_result);
            }
            warn!(
                target: LOG_TARGET,
                "Pledge data changed from previous execution of payload {}, re-executing payload.",
                payload.to_id(),
            );
        }

        let pledges = shard_pledges
            .iter()
            .map(|s| (s.shard_id, s.pledge.clone()))
            .collect::<HashMap<_, _>>();

        info!(target: LOG_TARGET, "[execute] Number of pledges: {}", pledges.len());
        for (k, v) in &pledges {
            // TODO: should be debug
            info!(
                target: LOG_TARGET,
                "[execute] shard: {}, pledge: {}",
                k,
                v.current_state.as_str()
            );
        }

        let consensus_context = ConsensusContext {
            current_epoch: epoch.as_u64(),
        };
        let finalize_result = self
            .payload_processor
            .process_payload(payload, pledges, consensus_context)?;
        Ok(finalize_result)
    }

    async fn get_leader(&self, node: &HotStuffTreeNode<TAddr, TPayload>) -> Result<TAddr, HotStuffError> {
        let epoch = self.epoch_manager.current_epoch().await?;
        let committee = self.epoch_manager.get_committee(epoch, node.shard()).await?;
        let leader = self.leader_strategy.get_leader(
            &committee,
            node.payload_id(),
            node.shard(),
            *self
                .current_leader_round
                .get(&(node.shard(), node.payload_id()))
                .unwrap_or(&0),
        );
        Ok(leader.clone())
    }

    fn create_vote(
        &self,
        node_hash: TreeNodeHash,
        shard_pledges: ShardPledgeCollection,
        payload_result: &TransactionResult,
        vn_shard_key: ShardId,
        vn_mmr: &ValidatorNodeMmr,
    ) -> Result<VoteMessage, HotStuffError> {
        let mut vote_msg = match payload_result {
            TransactionResult::Accept(ref accept) => {
                info!(
                    target: LOG_TARGET,
                    "💚 Vote to ACCEPT for node {}. Up substate(s): {}, down substate(s): {}",
                    node_hash,
                    accept.up_iter().count(),
                    accept.down_iter().count(),
                );
                VoteMessage::accept(node_hash, shard_pledges)
            },
            TransactionResult::Reject(ref reason) => {
                info!(target: LOG_TARGET, "⚔ Vote to REJECT payload: {}", reason);
                VoteMessage::reject(node_hash, shard_pledges, reason.into())
            },
        };

        vote_msg.sign_vote(&self.signing_service, vn_shard_key, vn_mmr)?;

        Ok(vote_msg)
    }

    /// A pacemaker beat has been triggered for a payload. If the leader has received enough NewViews, a Proposal is
    /// sent to replicas.
    async fn on_beat(&mut self, shard: ShardId, payload_id: PayloadId) -> Result<(), HotStuffError> {
        // TODO: the leader is only known after the leaf is determined
        // TODO: Review if this is correct. The epoch should be the same for the whole 3-chain

        let epoch = self.epoch_manager.current_epoch().await?;
        let committee = self.epoch_manager.get_committee(epoch, shard).await?;
        if committee.is_empty() {
            return Err(HotStuffError::NoCommitteeForShard { shard, epoch });
        }
        if self.is_leader(payload_id, shard, &committee)? {
            let min_required_new_views = committee.consensus_threshold();
            let num_new_views = self.get_newview_count_for(shard, payload_id);
            if num_new_views >= min_required_new_views {
                self.newview_message_counts.remove(&(shard, payload_id));
                self.leader_on_propose(shard, payload_id).await?;
            } else {
                info!(
                    target: LOG_TARGET,
                    "🔥 Waiting for more NEWVIEW messages ({}/{}) for shard {}, payload {}",
                    num_new_views,
                    min_required_new_views,
                    shard,
                    payload_id
                );
            }
        }
        Ok(())
    }

    async fn on_new_hs_message(
        &mut self,
        from: TAddr,
        msg: HotStuffMessage<TPayload, TAddr>,
    ) -> Result<(), HotStuffError> {
        match msg.message_type() {
            HotStuffMessageType::NewView => {
                let payload = msg
                    .new_view_payload()
                    .ok_or(HotStuffError::ReceivedNewViewWithoutPayload)?;
                self.leader_on_receive_new_view(from, msg.shard(), msg.high_qc().unwrap(), payload.clone())
                    .await?;
                // There should always be a payload, otherwise the leader
                // can't be determined
                self.on_beat(msg.shard(), payload.to_id()).await?;
            },
            HotStuffMessageType::Proposal => {
                let node = msg.node().ok_or(HotStuffError::RecvProposalMessageWithoutNode)?;
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
                    if let Some((payload, shard)) = msg {
                        if let Err(e) = self.on_next_sync_view(payload, shard).await {
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
                        // self.publish_event(HotStuffEvent::Failed(e.to_string()));
                        error!(target: LOG_TARGET, "Error while processing new hotstuff message (on_new_hs_message): {}", e);
                    }
                },
                Some((from, msg)) = self.rx_votes.recv() => {
                    debug!(target: LOG_TARGET, "Received vote from {}", from);
                    if let Err(e) = self.leader_on_receive_vote(from, msg).await {
                        error!(target: LOG_TARGET, "Error while processing vote (on_receive_vote): {}", e);
                    }
                },
                Some((from,msg)) = self.rx_recovery_message.recv() => {
                    debug!(target:LOG_TARGET, "Received recovery message from {}", from);
                    if let Err(e) = self.on_receive_recovery_message(from, msg).await {
                        error!(target: LOG_TARGET, "Error while processing recovery message (on_receive_recovery_message): {}", e);
                    }
                },
                Some(timeout) = self.pacemaker.on_timeout() => {
                    debug!(target:LOG_TARGET, "Received timeout message for {:?}", timeout);
                    if let Err(e) = self.on_pacemaker_trigger(timeout).await {
                        error!(target: LOG_TARGET, "Error while processing timeout message (on_timeout): {}", e);
                    }
                }
                _ = shutdown.wait() => {
                    info!(target: LOG_TARGET, "💤 Shutting down");
                    break;
                }
            }
        }
        Ok(())
    }
}

fn extract_changes_for_shards(
    shard_ids: &[ShardId],
    payload_id: PayloadId,
    diff: &SubstateDiff,
) -> Result<HashMap<ShardId, Vec<SubstateState>>, HotStuffError> {
    let mut changes = HashMap::<_, Vec<_>>::new();
    // down first, then up
    for (address, version) in diff.down_iter() {
        let shard_id = ShardId::from_address(address, *version);
        if shard_ids.contains(&shard_id) {
            changes
                .entry(shard_id)
                .or_default()
                .push(SubstateState::Down { deleted_by: payload_id });
        }
    }
    for (address, substate) in diff.up_iter() {
        let shard_id = ShardId::from_address(address, substate.version());
        if shard_ids.contains(&shard_id) {
            changes.entry(shard_id).or_default().push(SubstateState::Up {
                address: address.clone(),
                created_by: payload_id,
                data: substate.clone(),
            });
        }
    }

    Ok(changes)
}
