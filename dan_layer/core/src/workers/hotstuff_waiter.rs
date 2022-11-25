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

use std::collections::{HashMap, HashSet};

use log::*;
use tari_common_types::types::{PublicKey, Signature};
use tari_core::ValidatorNodeMmrHasherBlake256;
use tari_dan_common_types::{optional::Optional, Epoch, PayloadId, ShardId, SubstateState};
use tari_engine_types::commit_result::{FinalizeResult, RejectReason, TransactionResult};
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
        Committee,
        HotStuffMessage,
        HotStuffMessageType,
        HotStuffTreeNode,
        LeafNode,
        NodeHeight,
        ObjectPledge,
        Payload,
        QuorumCertificate,
        QuorumDecision,
        ShardVote,
        TreeNodeHash,
    },
    services::{
        epoch_manager::EpochManager,
        infrastructure_services::NodeAddressable,
        leader_strategy::LeaderStrategy,
        PayloadProcessor,
        SigningService,
    },
    storage::shard_store::{ShardStoreFactory, ShardStoreTransaction},
    workers::{events::HotStuffEvent, hotstuff_error::HotStuffError},
};

const LOG_TARGET: &str = "tari::dan_layer::hotstuff_waiter";

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
    /// Received vote messages
    rx_votes: Receiver<(TAddr, VoteMessage)>,
    /// Hotstuff messages that should be delivered to the leader
    tx_leader: Sender<(TAddr, HotStuffMessage<TPayload, TAddr>)>,
    /// Hotstuff messages that should be delivered to the replicas
    tx_broadcast: Sender<(HotStuffMessage<TPayload, TAddr>, Vec<TAddr>)>,
    /// Vote messages that should be delivered to the leader
    tx_vote_message: Sender<(VoteMessage, TAddr)>,
    /// HotstuffEvent channel
    tx_events: broadcast::Sender<HotStuffEvent>,
    /// The payload processor. This determines whether a payload proposal results in an accepted or rejected vote.
    payload_processor: TPayloadProcessor,
    /// Store used to persist consensus state.
    shard_store: TShardStore,
    /// Network-wide constants
    consensus_constants: ConsensusConstants,
}

impl<TPayload, TAddr, TLeaderStrategy, TEpochManager, TPayloadProcessor, TShardStore, TSigningService>
    HotStuffWaiter<TPayload, TAddr, TLeaderStrategy, TEpochManager, TPayloadProcessor, TShardStore, TSigningService>
where
    TPayload: Payload + 'static,
    TAddr: NodeAddressable + 'static,
    TLeaderStrategy: LeaderStrategy<TAddr> + 'static + Send + Sync,
    TEpochManager: EpochManager<TAddr> + 'static + Send + Sync,
    TPayloadProcessor: PayloadProcessor<TPayload> + 'static + Send + Sync,
    TShardStore: ShardStoreFactory<Addr = TAddr, Payload = TPayload> + 'static + Send + Sync,
    TSigningService: SigningService + Sync + Send + 'static,
{
    pub fn spawn(
        signing_service: TSigningService,
        public_key: TAddr,
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
            signing_service,
            public_key,
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
        signing_service: TSigningService,
        public_key: TAddr,
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
            signing_service,
            public_key,
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

    /// Step 1: A new payload has been received. The payload is persisted and a NewView is sent to the leader.
    async fn on_next_sync_view(&mut self, payload: TPayload, shard: ShardId) -> Result<(), HotStuffError> {
        let epoch = self.epoch_manager.current_epoch().await?;
        info!(
            target: LOG_TARGET,
            "🔥 [Epoch:{}] Send NEWVIEW for payload {} and shard {}",
            epoch,
            payload.to_id(),
            shard
        );
        let payload_id = payload.to_id();
        debug!(target: LOG_TARGET, "on_next_sync_view started: {:?}", payload_id);

        let new_view;
        {
            let mut tx = self.shard_store.create_tx()?;

            let high_qc = tx.get_high_qc_for(shard).optional()?.unwrap_or_else(|| {
                // TODO: sign genesis
                QuorumCertificate::genesis(epoch)
            });

            //  Save the payload, because we will need it when the proposal comes back
            tx.set_payload(payload.clone())?;
            tx.commit()?;
            // TODO: combine merkle proofs into one before sending
            new_view = HotStuffMessage::new_view(high_qc, shard, Some(payload));
        }

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

    /// Step 2: The leader receives a NewView from all committee members. The payload and QC are persisted.
    ///         Once $n - f$ NewViews have been received, a Proposal is sent to the replicas, see
    ///         [HotstuffWaiter::on_beat].
    async fn on_receive_new_view(
        &mut self,
        from: TAddr,
        shard: ShardId,
        qc: QuorumCertificate,
        payload: TPayload,
    ) -> Result<(), HotStuffError> {
        info!(
            target: LOG_TARGET,
            "🔥 Receive NewView for payload {}, shard {} and height {}",
            payload.to_id(),
            shard,
            qc.local_node_height()
        );

        let epoch = self.epoch_manager.current_epoch().await?;
        self.validate_from_committee(&from, epoch, shard).await?;
        // We dont expect signatures from other VNs at this point
        // TODO: should be 1, we expect the sender to sign their QC
        self.validate_qc(&qc, 0).await?;
        let mut tx = self.shard_store.create_tx()?;
        tx.update_high_qc(from, shard, qc)?;
        tx.set_payload(payload)?;
        tx.commit()?;
        Ok(())
    }

    /// Step 4: Sends a Proposal to replica. A new leaf node is created that builds
    /// on the previous tree or else a genesis node is created and proposed.
    async fn on_propose(&mut self, shard: ShardId, payload_id: PayloadId) -> Result<(), HotStuffError> {
        let epoch = self.epoch_manager.current_epoch().await?;

        let qc;
        let actual_payload;
        let parent_leaf_node;
        {
            let tx = self.shard_store.create_tx()?;

            // get_leaf_node returns the leaf node if it exists, otherwise it returns the genesis hash and height
            parent_leaf_node = tx.get_leaf_node(shard)?;
            qc = tx
                .get_high_qc_for(shard)
                .optional()?
                .unwrap_or_else(|| QuorumCertificate::genesis(epoch));
            actual_payload = tx.get_payload(&payload_id)?;
        }

        let involved_shards = actual_payload.involved_shards();
        let members = self
            .epoch_manager
            .get_committees(epoch, &involved_shards)
            .await?
            .into_iter()
            .flat_map(|allocation| allocation.committee.map(|c| c.members).unwrap_or_default())
            .collect();

        let leaf_node;
        {
            let mut tx = self.shard_store.create_tx()?;

            let parent = tx.get_node(parent_leaf_node.hash())?;
            let payload_height = if parent.payload_id() == payload_id {
                parent.payload_height() + NodeHeight(1)
            } else {
                NodeHeight(0)
            };
            info!(
                target: LOG_TARGET,
                "🔥 OnPropose for payload {} {} and shard {}", payload_height, payload_id, shard
            );
            if payload_height != NodeHeight(0) && parent.justify().local_node_hash() == qc.local_node_hash() {
                info!(
                    target: LOG_TARGET,
                    "Leaf node already has the same QC as the proposed QC, so we have already sent a proposal for \
                     this payload"
                );
                // We have already sent a proposal for this payload. Do nothing
                return Ok(());
            }

            if payload_height > NodeHeight(self.consensus_constants.hotstuff_rounds - 1) {
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
            let local_pledge = tx.pledge_object(shard, payload_id, change, parent_leaf_node.height())?;

            leaf_node = self.create_leaf(
                parent_leaf_node,
                shard,
                payload_id,
                actual_payload,
                qc,
                epoch,
                self.public_key.clone(),
                payload_height,
                Some(local_pledge),
            );
            tx.save_node(leaf_node.clone())?;
            tx.update_leaf_node(shard, *leaf_node.hash(), leaf_node.height())
                .map_err(|e| HotStuffError::UpdateLeafNode(e.to_string()))?;
            tx.commit()?;
        }

        self.tx_broadcast
            .send((HotStuffMessage::new_proposal(leaf_node, shard), members))
            .await
            .unwrap();

        Ok(())
    }

    /// Step 5: A replica receives a Proposal from the leader. The replica validates the proposal, executes the payload
    /// and sends a vote.
    #[allow(clippy::too_many_lines)]
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
            let mut tx = self.shard_store.create_tx()?;
            tx.set_payload(node_payload.clone())?;
            tx.commit()?;
            node_payload.clone()
        } else {
            let tx = self.shard_store.create_tx()?;
            tx.get_payload(&node.payload_id())?
        };
        let involved_shards = payload.involved_shards();
        // Find the shards that this node is responsible for out of all the shards involved
        let local_shards = self
            .epoch_manager
            .filter_to_local_shards(node.epoch(), &self.public_key, &involved_shards)
            .await?;

        let mut votes_to_send = vec![];
        let vns = self.epoch_manager.get_validator_nodes_per_epoch(node.epoch()).await?;
        let vn_mmr_leaf_index = vns
            .iter()
            .position(|vn| vn.public_key == self.public_key)
            .expect("The VN is not registered");

        let node_shard_key = self
            .epoch_manager
            .get_validator_shard_key(node.epoch(), self.public_key.clone())
            .await?;
        let vn_mmr = self.epoch_manager.get_validator_node_mmr(node.epoch()).await?;

        {
            let mut tx = self.shard_store.create_tx()?;
            tx.save_node(node.clone())?;
            let v_height = tx.get_last_voted_height(shard)?;
            // TODO: can also use the QC and committee to justify this....
            let (locked_node, locked_height) = tx.get_locked_node_hash_and_height(shard)?;
            if node.height() <= v_height ||
                (node.parent() != &locked_node && node.justify().local_node_height() <= locked_height)
            {
                error!(target: LOG_TARGET, "Received a proposal that is not valid");
                tx.commit()?;
                return Ok(());
            }

            tx.save_leader_proposals(node.shard(), node.payload_id(), node.payload_height(), node.clone())?;

            let mut leader_proposals = vec![];
            for s in &involved_shards {
                if let Some(leader_proposal) = tx.get_leader_proposals(node.payload_id(), node.payload_height(), *s)? {
                    leader_proposals.push(ShardVote {
                        shard_id: *s,
                        node_hash: *leader_proposal.hash(),
                        pledge: leader_proposal.local_pledge().cloned(),
                    });
                } else {
                    break;
                }
            }
            if leader_proposals.len() == involved_shards.len() {
                info!(
                    target: LOG_TARGET,
                    "🔥 Received enough proposals to vote on the message: {}",
                    leader_proposals.len()
                );
                // Execute the payload!
                let shard_pledges: HashMap<ShardId, Option<ObjectPledge>> = leader_proposals
                    .iter()
                    .map(|s| (s.shard_id, s.pledge.clone()))
                    .collect();

                // Execute for node 0. The rest we accept by default
                let finalize_result = if node.payload_height() == NodeHeight(0) {
                    let mut finalize_result = self.execute(shard_pledges.clone(), payload)?;
                    // validate that correct objects have been pledged.
                    let changes = extract_changes(node.payload_id(), &finalize_result)?;
                    // Change the vote if we do not have all the objects pledged
                    Self::validate_pledges(&shard_pledges, &mut finalize_result, changes);
                    finalize_result
                } else {
                    FinalizeResult::new(
                        node.payload_id().into_array().into(),
                        vec![],
                        TransactionResult::Accept(Default::default()),
                    )
                };

                // It may happen that we are involved in more than one committee, in which case send the votes to
                // each leader.
                for local_shard in local_shards {
                    dbg!("Can vote on the message");
                    let local_node = tx
                        .get_leader_proposals(node.payload_id(), node.payload_height(), local_shard)?
                        .unwrap();

                    tx.set_last_voted_height(local_shard, local_node.height())?;

                    let mut vote_msg = self.decide(
                        *local_node.hash(),
                        local_shard,
                        leader_proposals.clone(),
                        &finalize_result,
                    )?;

                    vote_msg.sign_vote(&self.signing_service, node_shard_key, &vn_mmr, vn_mmr_leaf_index as u64)?;

                    votes_to_send.push((vote_msg, local_node.proposed_by().clone()));
                }
            } else {
                info!(
                    target: LOG_TARGET,
                    "🔥 Not enough proposals to vote on the message, num proposals: {}, involved_shards: {}",
                    leader_proposals.len(),
                    involved_shards.len()
                );
            }
            tx.commit()?;
        }

        for msg in votes_to_send {
            self.tx_vote_message
                .send(msg)
                .await
                .map_err(|_| HotStuffError::SendError)?;
        }
        self.update_nodes(node.clone()).await?;
        Ok(())
    }

    /// Step 6: The leader receives votes from the local shard, and once it has enough ($n - f$) votes, it commits a
    /// high QC and sends the next round of proposals.
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
            if tx.has_vote_for(&from, msg.local_node_hash(), msg.shard())? {
                return Ok(());
            }

            node = tx.get_node(&msg.local_node_hash())?;

            if *node.proposed_by() != self.public_key {
                return Err(HotStuffError::NotTheLeader);
            }
        }

        let valid_committee = self.epoch_manager.get_committee(node.epoch(), node.shard()).await?;
        {
            let mut tx = self.shard_store.create_tx()?;
            if !valid_committee.contains(&from) {
                return Err(HotStuffError::ReceivedMessageFromNonCommitteeMember);
            }

            let total_votes = tx.save_received_vote_for(from, msg.local_node_hash(), msg.shard(), msg.clone())?;
            // Check for consensus
            dbg!(&valid_committee);
            dbg!(total_votes);
            if total_votes >= valid_committee.consensus_threshold() {
                let mut different_votes = HashMap::new();
                for vote in tx.get_received_votes_for(msg.local_node_hash(), msg.shard())? {
                    let entry = different_votes.entry(vote.get_all_nodes_hash()).or_insert(vec![]);
                    entry.push(vote);
                }

                // Check that there is sufficient votes for a single set of nodes that we can use to generate a qc
                for (_hash, votes) in different_votes {
                    if votes.len() >= valid_committee.consensus_threshold() {
                        let validators_metadata = votes.iter().map(|v| v.validator_metadata().clone()).collect();

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
                            validators_metadata,
                        );
                        tx.update_high_qc(node.proposed_by().clone(), msg.shard(), qc)?;

                        // Should be the pace maker actually
                        on_beat_future = Some(self.on_beat(msg.shard(), node.payload_id()));
                        break;
                    }
                }
            }

            // commit the transaction
            tx.commit()?;
            // drop tx
        }

        if let Some(on_beat) = on_beat_future {
            on_beat.await?;
        }
        Ok(())
    }

    /// A pacemaker beat has been triggered for a payload. If the leader has received enough NewViews, a Proposal is
    /// sent to replicas.
    async fn on_beat(&mut self, shard: ShardId, payload_id: PayloadId) -> Result<(), HotStuffError> {
        // TODO: the leader is only known after the leaf is determined
        // TODO: Review if this is correct. The epoch should stay the same for all epochs

        let epoch = self.epoch_manager.current_epoch().await?;
        let committee = self.epoch_manager.get_committee(epoch, shard).await?;
        if self.is_leader(payload_id, shard, &committee)? {
            let min_required_new_views = committee.consensus_threshold();
            let num_new_views = self.count_new_views_for(shard)?;
            if num_new_views >= min_required_new_views {
                self.on_propose(shard, payload_id).await?;
            } else {
                info!(
                    target: LOG_TARGET,
                    "🔥 Waiting for more NEWVIEW messages ({}/{})", num_new_views, min_required_new_views
                );
            }
        }
        Ok(())
    }

    fn count_new_views_for(&self, shard: ShardId) -> Result<usize, HotStuffError> {
        let tx = self.shard_store.create_tx()?;
        let count = tx.count_high_qc_for(shard)?;
        Ok(count)
    }

    fn create_leaf(
        &self,
        parent: LeafNode,
        shard: ShardId,
        payload_id: PayloadId,
        payload: TPayload,
        qc: QuorumCertificate,
        epoch: Epoch,
        leader: TAddr,
        payload_height: NodeHeight,
        local_pledge: Option<ObjectPledge>,
    ) -> HotStuffTreeNode<TAddr, TPayload> {
        // We only need to send the payload for the first round, otherwise we choose not to include it to reduce message
        // size.
        let maybe_payload = if payload_height.as_u64() == 0 {
            Some(payload)
        } else {
            None
        };
        HotStuffTreeNode::new(
            *parent.hash(),
            shard,
            parent.height() + NodeHeight(1),
            payload_id,
            maybe_payload,
            payload_height,
            local_pledge,
            epoch,
            leader,
            qc,
        )
    }

    fn is_leader(
        &self,
        payload: PayloadId,
        shard: ShardId,
        committee: &Committee<TAddr>,
    ) -> Result<bool, HotStuffError> {
        Ok(self
            .leader_strategy
            .is_leader(&self.public_key, committee, payload, shard, 0))
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
                    md.merkle_leaf_index as usize,
                )
                .map_err(|_| HotStuffError::InvalidQuorumCertificate("invalid merkle proof".to_string()))?;
        }

        // all signers must be included in the epoch commitee for the shard
        let committee = self.epoch_manager.get_committee(qc.epoch(), qc.shard()).await?;
        let commitee_set = committee.members.iter().map(|m| m.as_bytes()).collect::<HashSet<_>>();
        let all_signers_are_in_committee = signers_set.iter().all(|s| commitee_set.contains(s));
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
        let vote = VoteMessage::new(
            qc.local_node_hash(),
            qc.shard(),
            *qc.decision(),
            qc.all_shard_nodes().to_vec(),
        );
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

    /// Performs the requisite state updates for a given tree node.
    /// If the tree node is below height 2, nothing needs to happen.
    /// If the tree node is at height 2, we lock the parent node.
    /// And if at height 3 we execute and commit the state.
    async fn update_nodes(&mut self, node: HotStuffTreeNode<TAddr, TPayload>) -> Result<(), HotStuffError> {
        let shard = node.shard();

        if node.justify().local_node_hash() == TreeNodeHash::zero() {
            dbg!("Node is parented to genesis, no need to update");
            return Ok(());
        }

        let mut tx = self.shard_store.create_tx()?;
        tx.update_high_qc(node.proposed_by().clone(), shard, node.justify().clone())?;

        let b_two = tx.get_node(&node.justify().local_node_hash())?;
        if b_two.justify().local_node_hash() == TreeNodeHash::zero() {
            dbg!("b one is genesis, nothing to do");
            return Ok(());
        }
        let b_one = tx.get_node(&b_two.justify().local_node_hash())?;

        let (_b_lock, b_lock_height) = tx.get_locked_node_hash_and_height(shard)?;
        if b_one.height() > b_lock_height {
            info!(target: LOG_TARGET, "Updating locked node to: {:?}", b_one.hash());
            tx.set_locked(shard, *b_one.hash(), b_one.height())?;
        }

        if node.justify().payload_height() == NodeHeight(self.consensus_constants.hotstuff_rounds - 2) {
            let payload = tx.get_payload(&node.payload_id())?;
            let shard_pledges = node
                .justify()
                .all_shard_nodes()
                .iter()
                .map(|s| (s.shard_id, s.pledge.clone()))
                .collect();
            // TODO: Perhaps we should extract the data from the justify rather....
            let finalize_result = self.execute(shard_pledges, payload)?;
            let changes = extract_changes(node.payload_id(), &finalize_result)?;
            info!(
                target: LOG_TARGET,
                "payload changeset: {}",
                serde_json::to_string(&changes).unwrap()
            );
            let qc = node.justify().clone();
            self.on_commit(&node, &changes, &mut tx)?;
            self.publish_event(HotStuffEvent::OnFinalized(Box::new(qc), finalize_result));
        }
        tx.commit()?;

        Ok(())
    }

    /// Commits the changeset and node including all parent nodes if not already done so.
    fn on_commit(
        &mut self,
        node: &HotStuffTreeNode<TAddr, TPayload>,
        changes: &HashMap<ShardId, Vec<SubstateState>>,
        tx: &mut TShardStore::Transaction,
    ) -> Result<(), HotStuffError> {
        let shard = node.shard();
        if tx.get_last_executed_height(shard)? < node.height() {
            info!(
                target: LOG_TARGET,
                "🔥 OnCommit for payload {} and shard {}",
                node.payload_id(),
                shard,
            );

            if node.parent() != &TreeNodeHash::zero() {
                let parent = tx.get_node(node.parent())?;
                let changes = extract_changes_from_justify(parent.justify())?;
                self.on_commit(&parent, &changes, tx)?;
            }

            if node.justify().payload_height() == NodeHeight(self.consensus_constants.hotstuff_rounds - 2) &&
                *node.justify().decision() == QuorumDecision::Accept
            {
                tx.save_substate_changes(changes, node)?;
            }
            tx.set_last_executed_height(shard, node.height())?;
        }
        Ok(())
    }

    fn execute(
        &self,
        shard_pledges: HashMap<ShardId, Option<ObjectPledge>>,
        payload: TPayload,
    ) -> Result<FinalizeResult, HotStuffError> {
        info!(target: LOG_TARGET, "🔥 Executing payload: {}", payload.to_id());
        let finalize = self.payload_processor.process_payload(payload, shard_pledges)?;
        Ok(finalize)
    }

    fn validate_proposal(&self, node: &HotStuffTreeNode<TAddr, TPayload>) -> Result<(), HotStuffError> {
        if node.payload_height() == NodeHeight(0) ||
            (node.payload_id() == node.justify().payload_id() &&
                node.payload_height() == node.justify().payload_height() + NodeHeight(1))
        {
            let max_node_height = NodeHeight(self.consensus_constants.hotstuff_rounds - 1);
            if node.payload_height() > max_node_height {
                return Err(HotStuffError::PayloadHeightIsTooHigh {
                    actual: node.payload_height(),
                    max: max_node_height,
                });
            }
            Ok(())
        } else {
            Err(HotStuffError::NodePayloadDoesNotMatchJustifyPayload)
        }
    }

    fn validate_pledges(
        shard_pledges: &HashMap<ShardId, Option<ObjectPledge>>,
        finalize_result: &mut FinalizeResult,
        changes: HashMap<ShardId, Vec<SubstateState>>,
    ) {
        for (shard_changed, substates) in changes {
            // TODO: Is this statement correct?
            // If there are multiple changes to the substate, only the first one needs to be pledged for.
            if let Some(substate) = substates.first() {
                let pledged_object = shard_pledges.get(&shard_changed);
                if let Some(Some(pledge)) = pledged_object {
                    match substate {
                        SubstateState::DoesNotExist => match pledge.current_state {
                            SubstateState::DoesNotExist => {
                                debug!(
                                    target: LOG_TARGET,
                                    "Pledge requires object to not exist, and it does not exist - ok"
                                );
                            },
                            _ => {
                                finalize_result.result = TransactionResult::Reject(RejectReason::ShardNotPledged(
                                    format!("Shard {} was required to not exist, but it does", shard_changed),
                                ));
                                break;
                            },
                        },
                        SubstateState::Up { .. } => match pledge.current_state {
                            SubstateState::DoesNotExist => {
                                debug!(
                                    target: LOG_TARGET,
                                    "Pledge requires object to not exist and will be UPPED, and it does not exist - ok"
                                );
                            },
                            _ => {
                                finalize_result.result =
                                    TransactionResult::Reject(RejectReason::ShardNotPledged(format!(
                                        "Shard {} was required to not exist, but it is {}",
                                        shard_changed,
                                        pledge.current_state.as_str(),
                                    )));
                                break;
                            },
                        },
                        SubstateState::Down { .. } => match pledge.current_state {
                            SubstateState::Up { .. } => {
                                debug!(
                                    target: LOG_TARGET,
                                    "Pledge requires object to be DOWNED, and it is UP - Ok"
                                );
                            },
                            _ => {
                                finalize_result.result =
                                    TransactionResult::Reject(RejectReason::ShardNotPledged(format!(
                                        "Shard {} was required to be up, but it is {}",
                                        shard_changed,
                                        pledge.current_state.as_str(),
                                    )));
                                break;
                            },
                        },
                    }
                } else {
                    finalize_result.result = TransactionResult::Reject(RejectReason::ShardNotPledged(format!(
                        "Shard {} was not pledged",
                        shard_changed
                    )));
                    break;
                }
            } else {
                finalize_result.result = TransactionResult::Reject(RejectReason::ShardNotPledged(format!(
                    "Shard {} had no substate changes - this is not correct",
                    shard_changed
                )));
                break;
            }
        }
    }

    fn decide(
        &self,
        local_node: TreeNodeHash,
        local_shard: ShardId,
        votes: Vec<ShardVote>,
        finalize_result: &FinalizeResult,
    ) -> Result<VoteMessage, HotStuffError> {
        let vote_msg = match finalize_result.result {
            TransactionResult::Accept(ref accept) => {
                info!(
                    target: LOG_TARGET,
                    "💚 Vote to ACCEPT payload. Up substate(s): {}, down substate(s): {}",
                    accept.up_iter().count(),
                    accept.down_iter().count(),
                );
                VoteMessage::accept(local_node, local_shard, votes)
            },
            TransactionResult::Reject(ref reason) => {
                match reason {
                    RejectReason::ShardNotPledged(msg) => {
                        info!(target: LOG_TARGET, "⚔ Vote to REJECT payload: {}", msg);
                    },
                    RejectReason::ExecutionFailure(msg) => {
                        info!(target: LOG_TARGET, "Payload execution failure: {}", msg);
                    },
                }
                VoteMessage::reject(local_node, local_shard, votes, reason)
            },
        };

        Ok(vote_msg)
    }

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
) -> Result<HashMap<ShardId, Vec<SubstateState>>, HotStuffError> {
    let mut changes = HashMap::new();
    match finalize.result {
        TransactionResult::Accept(ref diff) => {
            // down first, then up
            for address in diff.down_iter() {
                changes
                    .entry(ShardId::from_address(address))
                    .or_insert(vec![])
                    .push(SubstateState::Down { deleted_by: payload_id });
            }
            for (address, substate) in diff.up_iter() {
                changes
                    .entry(ShardId::from_address(address))
                    .or_insert(vec![])
                    .push(SubstateState::Up {
                        created_by: payload_id,
                        data: substate.clone(),
                    });
            }
        },
        TransactionResult::Reject(ref reason) => return Err(HotStuffError::TransactionRejected(reason.clone())),
    }

    Ok(changes)
}

fn extract_changes_from_justify(
    justify: &QuorumCertificate,
) -> Result<HashMap<ShardId, Vec<SubstateState>>, HotStuffError> {
    let mut changes = HashMap::new();
    for a in justify.all_shard_nodes() {
        if let Some(ref p) = a.pledge {
            changes
                .entry(p.shard_id)
                .or_insert(vec![])
                .push(p.current_state.clone());
        }
    }

    Ok(changes)
}
