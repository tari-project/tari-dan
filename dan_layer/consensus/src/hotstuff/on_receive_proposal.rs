//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

// (New, true) ----(cmd:Prepare) ---> (Prepared, true) -----cmd:LocalPrepared ---> (LocalPrepared, false)
// ----[foreign:LocalPrepared]--->(LocalPrepared, true) ----cmd:AllPrepare ---> (AllPrepared, true) ---cmd:Accept --->
// Complete

use std::{collections::HashSet, ops::DerefMut};

use log::*;
use tari_dan_common_types::{
    committee::{Committee, CommitteeShard},
    optional::Optional,
    NodeHeight,
};
use tari_dan_storage::{
    consensus_models::{
        Block,
        BlockId,
        Command,
        Decision,
        ExecutedTransaction,
        LastExecuted,
        LastVoted,
        LeafBlock,
        LockedBlock,
        QuorumDecision,
        SubstateLockFlag,
        SubstateRecord,
        TransactionPool,
        TransactionPoolStage,
        TransactionRecord,
    },
    StateStore,
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
};
use tari_epoch_manager::EpochManagerReader;
use tari_transaction::Transaction;
use tokio::sync::{broadcast, mpsc};

use crate::{
    hotstuff::{
        common::update_high_qc,
        error::HotStuffError,
        event::HotstuffEvent,
        on_beat::OnBeat,
        ProposalValidationError,
    },
    messages::{HotstuffMessage, ProposalMessage, RequestMissingTransactionsMessage, VoteMessage},
    traits::{ConsensusSpec, LeaderStrategy, StateManager, VoteSignatureService},
};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::on_receive_proposal";

pub struct OnReceiveProposalHandler<TConsensusSpec: ConsensusSpec> {
    validator_addr: TConsensusSpec::Addr,
    store: TConsensusSpec::StateStore,
    epoch_manager: TConsensusSpec::EpochManager,
    vote_signing_service: TConsensusSpec::VoteSignatureService,
    leader_strategy: TConsensusSpec::LeaderStrategy,
    state_manager: TConsensusSpec::StateManager,
    transaction_pool: TransactionPool<TConsensusSpec::StateStore>,
    tx_leader: mpsc::Sender<(TConsensusSpec::Addr, HotstuffMessage)>,
    tx_events: broadcast::Sender<HotstuffEvent>,
    tx_broadcast: mpsc::Sender<(Committee<TConsensusSpec::Addr>, HotstuffMessage)>,
    on_beat: OnBeat,
}

impl<TConsensusSpec> OnReceiveProposalHandler<TConsensusSpec>
where TConsensusSpec: ConsensusSpec
{
    pub fn new(
        validator_addr: TConsensusSpec::Addr,
        store: TConsensusSpec::StateStore,
        epoch_manager: TConsensusSpec::EpochManager,
        vote_signing_service: TConsensusSpec::VoteSignatureService,
        leader_strategy: TConsensusSpec::LeaderStrategy,
        state_manager: TConsensusSpec::StateManager,
        transaction_pool: TransactionPool<TConsensusSpec::StateStore>,
        tx_leader: mpsc::Sender<(TConsensusSpec::Addr, HotstuffMessage)>,
        tx_events: broadcast::Sender<HotstuffEvent>,
        tx_broadcast: mpsc::Sender<(Committee<TConsensusSpec::Addr>, HotstuffMessage)>,
        on_beat: OnBeat,
    ) -> Self {
        Self {
            validator_addr,
            store,
            epoch_manager,
            vote_signing_service,
            leader_strategy,
            state_manager,
            transaction_pool,
            tx_leader,
            tx_events,
            tx_broadcast,
            on_beat,
        }
    }

    pub async fn handle(&self, from: TConsensusSpec::Addr, message: ProposalMessage) -> Result<(), HotStuffError> {
        let ProposalMessage { block } = message;

        let local_committee = self.epoch_manager.get_local_committee(block.epoch()).await?;
        let num_committees = self.epoch_manager.get_num_committees(block.epoch()).await?;
        let our_bucket = self
            .epoch_manager
            .get_our_validator_node(block.epoch())
            .await?
            .shard_key
            .to_committee_bucket(num_committees);

        if block.proposed_by().to_committee_bucket(num_committees) == our_bucket {
            debug!(
                target: LOG_TARGET,
                "🔥 Receive LOCAL PROPOSAL for block {}, parent {}, height {} from {}",
                block.id(),
                block.parent(),
                block.height(),
                from,
            );

            self.handle_local_proposal(from, local_committee, block).await
        } else {
            debug!(
                target: LOG_TARGET,
                "🔥 Receive FOREIGN PROPOSAL for block {}, parent {}, height {} from {}",
                block.id(),
                block.parent(),
                block.height(),
                from,
            );

            self.handle_foreign_proposal(from, &local_committee, block).await
        }
    }

    // Returns the indices in foreign committee to whom I should send the block, so we guarantee that at least one
    // honest node receives it in the foreign committee. So we need to send it to at least f+1 node in the foreign
    // committee. But also take into consideration that our committee can have f dishonest nodes.
    fn to_whom_should_i_resend_the_block(
        &self,
        my_index: usize,
        my_committee_size: usize,
        foreign_committee_size: usize,
    ) -> Vec<usize> {
        if my_index == 0 {
            // I'm leader, I already sent it to everyone
            vec![]
        } else if foreign_committee_size / 3 + my_committee_size / 3 < my_committee_size {
            // We can do 1 to 1 mapping, but am I one that should be sending it?
            if foreign_committee_size / 3 + my_committee_size / 3 + 1 > my_index {
                vec![my_index]
            } else {
                vec![]
            }
        } else {
            // We can't do 1 to 1 mapping, the foreign committee is too big (more than 2 times), so now we have need
            // 1 to N mapping.
            // Lets the size of the committee be the 3f+1 and the foreign committee 3g+1.
            // We know that f+g+1 > 3f+1 (that's above)
            // So now we need to send more than 1 message per node. If we send n messages per node from x nodes, then we
            // need (x-f)*n > g, because f nodes can be faulty, and we need to hit at least one honest node. So the
            // smallest n is n=g/(x-f)+1. If we send it from the whole committee then n=g/(2f+1)+1. Ok but now, due to
            // rounding, we don't have to send it from all the nodes. We just need to satify the (x-f)*n>g. So now we
            // compute the x, x=g/n+f+1.
            let my_f = (my_committee_size - 1) / 3;
            let foreign_f = (foreign_committee_size - 1) / 3;
            let n = foreign_f / (my_committee_size - my_f) + 1;
            let nodes = foreign_f / n + 1 + my_f;
            // So now we have 1 to N mapping, nodes will each send n messages
            if my_index < nodes {
                ((my_index * n)..((my_index + 1) * n))
                    .map(|i| i % foreign_committee_size)
                    .collect()
            } else {
                vec![]
            }
        }
    }

    async fn send_block_to_foreign_committees(&self, block: Block) -> Result<(), HotStuffError> {
        // We now have a valid block and we may be responsible to send it to the foreign committees
        let txs = self.store.with_read_tx(|tx| {
            let prepared_iter = block
                .commands()
                .iter()
                .filter_map(|cmd| cmd.local_prepared())
                .map(|t| &t.id);
            ExecutedTransaction::get_any(tx, prepared_iter)
        })?;
        let involved_shards = txs
            .iter()
            .flat_map(|tx| tx.transaction().involved_shards_iter().copied())
            .collect::<HashSet<_>>();

        let num_committees = self.epoch_manager.get_num_committees(block.epoch()).await?;
        let validator = self.epoch_manager.get_our_validator_node(block.epoch()).await?;
        let local_bucket = validator.shard_key.to_committee_bucket(num_committees);

        let non_local_buckets = involved_shards
            .into_iter()
            .map(|shard| shard.to_committee_bucket(num_committees))
            .filter(|bucket| *bucket != local_bucket)
            .collect::<HashSet<_>>();

        let non_local_committees = self
            .epoch_manager
            .get_committees_by_buckets(block.epoch(), non_local_buckets)
            .await?;
        let local_committee = self.epoch_manager.get_local_committee(block.epoch()).await?;

        let mut committee = Vec::new();
        let local_leader_index = self
            .leader_strategy
            .calculate_leader(&local_committee, block.id(), block.height()) as usize;
        let my_index = local_committee
            .members
            .iter()
            .position(|member| *member == validator.address)
            .expect("I should be part of my local committee");
        // Recompute my_index to be zero-based from leader position.
        let my_index = (my_index + local_committee.len() - local_leader_index) % local_committee.len();

        for non_local_commitee in non_local_committees.values() {
            let send_to =
                self.to_whom_should_i_resend_the_block(my_index, local_committee.len(), non_local_commitee.len());
            let foreign_leader_index =
                self.leader_strategy
                    .calculate_leader(non_local_commitee, block.id(), block.height()) as usize;
            committee.append(
                &mut send_to
                    .into_iter()
                    .map(|index| {
                        non_local_commitee.members[(index + foreign_leader_index) % non_local_commitee.len()].clone()
                    })
                    .collect(),
            )
        }
        if committee.is_empty() {
            Ok(())
        } else {
            self.tx_broadcast
                .send((
                    Committee::new(committee),
                    HotstuffMessage::Proposal(ProposalMessage { block }),
                ))
                .await
                .map_err(|_| HotStuffError::InternalChannelClosed {
                    context: "syncing local block to foreign committees",
                })
        }
    }

    async fn handle_local_proposal(
        &self,
        from: TConsensusSpec::Addr,
        local_committee: Committee<TConsensusSpec::Addr>,
        block: Block,
    ) -> Result<(), HotStuffError> {
        // First save the block in one db transaction
        self.store.with_write_tx(|tx| {
            self.validate_local_proposed_block(&mut *tx, &from, &block)?;
            // Insert the block if it doesnt already exist
            block.justify().save(tx)?;
            block.save(tx)?;
            Ok::<_, HotStuffError>(())
        })?;

        self.send_block_to_foreign_committees(block.clone()).await?;

        if self.block_has_missing_transaction(&local_committee, &block).await? {
            Ok(())
        } else {
            self.process_block(&local_committee, &block).await
        }
    }

    async fn block_has_missing_transaction(
        &self,
        local_committee: &Committee<TConsensusSpec::Addr>,
        block: &Block,
    ) -> Result<bool, HotStuffError> {
        let mut missing_tx_ids = Vec::new();
        let mut awaiting_execution = Vec::new();
        // TODO(perf): n queries
        self.store.with_read_tx(|tx| {
            for tx_id in block.all_transaction_ids() {
                match TransactionRecord::get(tx, tx_id).optional()? {
                    Some(tx) => {
                        // If execution is in progress, we need to note down the transactions without requesting them
                        if tx.result.is_none() {
                            awaiting_execution.push(*tx_id);
                        }
                    },
                    None => missing_tx_ids.push(*tx_id),
                }
            }
            Ok::<_, HotStuffError>(())
        })?;

        if missing_tx_ids.is_empty() && awaiting_execution.is_empty() {
            return Ok(false);
        }

        info!(
            target: LOG_TARGET,
            "🔥 Block {} has {} missing transactions and {} awaiting execution", block.id(), missing_tx_ids.len(), awaiting_execution.len(),
        );

        self.store.with_write_tx(|tx| {
            tx.insert_missing_transactions(block.id(), missing_tx_ids.iter().chain(&awaiting_execution))
        })?;

        if !missing_tx_ids.is_empty() {
            let leader = self
                .leader_strategy
                .get_leader(local_committee, block.id(), block.height());
            self.send_to_leader(
                leader,
                HotstuffMessage::RequestMissingTransactions(RequestMissingTransactionsMessage {
                    block_id: *block.id(),
                    epoch: block.epoch(),
                    transactions: missing_tx_ids,
                }),
            )
            .await?;
        }

        Ok(true)
    }

    pub async fn reprocess_block(&self, block_id: &BlockId) -> Result<(), HotStuffError> {
        let block = self.store.with_read_tx(|tx| Block::get(tx, block_id))?;
        let local_committee = self.epoch_manager.get_local_committee(block.epoch()).await?;
        self.process_block(&local_committee, &block).await
    }

    async fn process_block(
        &self,
        local_committee: &Committee<<TConsensusSpec as ConsensusSpec>::Addr>,
        block: &Block,
    ) -> Result<(), HotStuffError> {
        let local_committee_shard = self.epoch_manager.get_local_committee_shard(block.epoch()).await?;
        let maybe_decision = self.store.with_write_tx(|tx| {
            let should_vote = self.should_vote(&mut *tx, block)?;

            let mut maybe_decision = None;
            if should_vote {
                maybe_decision = self.decide_what_to_vote(tx, block, &local_committee_shard)?;
            }

            self.update_nodes(tx, block, &local_committee_shard)?;
            Ok::<_, HotStuffError>(maybe_decision)
        })?;

        if let Some(decision) = maybe_decision {
            let vote = self.generate_vote_message(block, decision).await?;
            debug!(
                target: LOG_TARGET,
                "🔥 Send {:?} VOTE for block {}, parent {}, height {}",
                decision,
                block.id(),
                block.parent(),
                block.height(),
            );
            self.send_vote_to_leader(local_committee, vote, block.height()).await?;
        }

        Ok(())
    }

    async fn handle_foreign_proposal(
        &self,
        from: TConsensusSpec::Addr,
        local_committee: &Committee<TConsensusSpec::Addr>,
        block: Block,
    ) -> Result<(), HotStuffError> {
        if self.store.with_read_tx(|tx| block.justify().exists(tx))? {
            // We already seen this block. And the block we saw was valid.
            return Ok(());
        }

        let committee_shard = self
            .epoch_manager
            .get_committee_shard(block.epoch(), *block.proposed_by())
            .await?;
        self.validate_proposed_block(&from, &block)?;
        self.store
            .with_write_tx(|tx| self.on_receive_foreign_block(tx, &block, &committee_shard))?;

        // If we received the foreign proposal, we send it to the leader (if we are not the leader), the leader then
        // redistributes the block to all other nodes. This way if the leader is not faulty O(n) messages will be send
        // around. If the leader doesn't have the message it will take 2 delta (if the delta time is the maximum latency
        // between any two nodes) to have it everywhere. If the leader has the message already it will be just 1 delta.
        // Worst case scenario is when we have f faulty nodes, and 2f honest nodes have the message and 1 node doesnt,
        // but he is the (f+1)th leader. In this case we send exactly 2f*f+2f+3f messages around. 2f*f to the
        // faulty leaders, 2f to the honest leader, and 3f from the leader.
        let leader = self.store.with_read_tx(|tx| {
            let leaf_block = LeafBlock::get(tx, block.epoch())?;
            Ok::<_, HotStuffError>(
                self.leader_strategy
                    .get_leader_for_next_block(local_committee, &leaf_block.block_id, leaf_block.height)
                    .clone(),
            )
        })?;
        if leader == self.validator_addr {
            // We are the leader, so we distribute the block within the local committee (we didn't do it yet)
            // If there leader is malicious and doesn't redistribute the block we should handle the redistribution again
            // on leader rotation, from all the nodes that have this block. Because the next leader may not have this
            // block.
            self.tx_broadcast
                .send((
                    local_committee.clone(),
                    HotstuffMessage::Proposal(ProposalMessage { block }),
                ))
                .await
                .map_err(|_| HotStuffError::InternalChannelClosed {
                    context: "Redistributing foreign block",
                })?;
        } else {
            // We are not the leader, so we send the block to the leader
            self.send_to_leader(
                &leader,
                HotstuffMessage::Proposal(ProposalMessage { block: block.clone() }),
            )
            .await?;
        }

        // We could have ready transactions at this point, so if we're the leader for the next block we can propose
        self.on_beat.beat();

        Ok(())
    }

    fn on_receive_foreign_block(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        block: &Block,
        foreign_committee_shard: &CommitteeShard,
    ) -> Result<(), HotStuffError> {
        // Save the QCs if it doesnt exist already, we'll reference the QC in subsequent blocks
        block.justify().save(tx)?;

        // TODO(perf): n queries
        for cmd in block.commands() {
            let Some(t) = cmd.local_prepared() else {
                continue;
            };
            let Some(mut tx_rec) = self.transaction_pool.get(tx, &t.id).optional()? else {
                continue;
            };

            if tx_rec.stage().is_all_prepared() || tx_rec.stage().is_some_prepared() {
                warn!(
                    target: LOG_TARGET,
                    "⚠️ Foreign proposal received after transaction {} is {}. Ignoring.",
                    tx_rec.transaction.id, tx_rec.stage
                );
                continue;
            }

            tx_rec.update_evidence(tx, foreign_committee_shard, *block.justify().id())?;
            let change_to_abort = cmd.decision().is_abort() && tx_rec.original_decision().is_commit();
            if change_to_abort {
                info!(
                    target: LOG_TARGET,
                    "⚠️ Foreign shard ABORT {}. Update decision to ABORT",
                    tx_rec.transaction.id
                );
                tx_rec.set_pending_decision(tx, Decision::Abort)?;
            }

            // If all shards are complete and we've already received our LocalPrepared, we can set out LocalPrepared
            // transaction as ready to propose ACCEPT. If we have not received the local LocalPrepared, the transition
            // will happen when we receive the local block.
            if tx_rec.stage().is_local_prepared() && tx_rec.transaction.evidence.all_shards_complete() {
                tx_rec.transition(tx, TransactionPoolStage::LocalPrepared, true)?;
            }
        }

        Ok(())
    }

    async fn send_to_leader(
        &self,
        leader: &TConsensusSpec::Addr,
        message: HotstuffMessage,
    ) -> Result<(), HotStuffError> {
        self.tx_leader
            .send((leader.clone(), message))
            .await
            .map_err(|_| HotStuffError::InternalChannelClosed {
                context: "tx_leader in OnReceiveProposalHandler::send_to_leader",
            })
    }

    async fn send_vote_to_leader(
        &self,
        local_committee: &Committee<TConsensusSpec::Addr>,
        vote: VoteMessage,
        height: NodeHeight,
    ) -> Result<(), HotStuffError> {
        let leader = self
            .leader_strategy
            .get_leader_for_next_block(local_committee, &vote.block_id, height);
        self.tx_leader
            .send((leader.clone(), HotstuffMessage::Vote(vote)))
            .await
            .map_err(|_| HotStuffError::InternalChannelClosed {
                context: "tx_leader in OnReceiveProposalHandler::send_vote_to_leader",
            })
    }

    #[allow(clippy::too_many_lines)]
    fn decide_what_to_vote(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        block: &Block,
        local_committee_shard: &CommitteeShard,
    ) -> Result<Option<QuorumDecision>, HotStuffError> {
        block.as_last_voted().set(tx)?;

        for cmd in block.commands() {
            let mut tx_rec = self.transaction_pool.get(tx, cmd.transaction_id())?;
            // TODO: we probably need to provide the all/some of the QCs referenced in local transactions as
            //       part of the proposal DanMessage so that there is no race condition between receiving the
            //       proposed block and receiving the foreign proposals
            tx_rec.update_evidence(tx, local_committee_shard, *block.justify().id())?;

            debug!(
                target: LOG_TARGET,
                "🔥 vote for block {} {}. Cmd: {}",
                block.id(),
                block.height(),
                cmd,
            );
            match cmd {
                Command::Prepare(t) => {
                    if !tx_rec.stage().is_new() {
                        warn!(
                            target: LOG_TARGET,
                            "❌ Stage disagreement for block {}. Leader proposed Prepare, local stage {}",
                            block.id(),
                            tx_rec.stage()
                        );
                        return Ok(None);
                    }
                    if tx_rec.original_decision() == t.decision {
                        if tx_rec.original_decision().is_commit() {
                            let transaction = ExecutedTransaction::get(tx.deref_mut(), cmd.transaction_id())?;
                            // Lock all inputs for the transaction as part of LocalPrepare
                            if !self.lock_inputs(tx, transaction.transaction(), local_committee_shard)? {
                                // Unable to lock all inputs - abstain? or vote reject?
                                warn!(
                                    target: LOG_TARGET,
                                    "❌ Unable to lock inputs for block {}. Leader proposed {}, we decided {}",
                                    block.id(),
                                    t.decision,
                                    tx_rec.original_decision()
                                );
                                return Ok(None);
                            }
                        }

                        tx_rec.transition(tx, TransactionPoolStage::Prepared, true)?;
                    } else {
                        // If we disagree with any local decision we abstain from voting
                        warn!(
                            target: LOG_TARGET,
                            "❌ Prepare decision disagreement for block {}. Leader proposed {}, we decided {}",
                            block.id(),
                            t.decision,
                            tx_rec.original_decision()
                        );
                        return Ok(None);
                    }
                },
                Command::LocalPrepared(t) => {
                    // Happy path: We've validated all the QCs and therefore are convinced that everyone also Prepared.
                    // We only mark the next step (Accept) as ready to propose once all shards have reported
                    // LocalPrepared.

                    if !tx_rec.stage().is_prepared() {
                        warn!(
                            target: LOG_TARGET,
                            "❌ Stage disagreement in block {} for transaction {}. Leader proposed LocalPrepared, but we have not prepared",
                            block.id(),
                            tx_rec.transaction_id()
                        );
                        return Ok(None);
                    }
                    // We check that the committee decision is different from the local decision.
                    if tx_rec.original_decision() != t.decision {
                        warn!(
                            target: LOG_TARGET,
                            "❌ LocalPrepared decision disagreement for block {}. Leader proposed {}, we decided {}",
                            block.id(),
                            t.decision,
                            tx_rec.transaction.decision
                        );
                        return Ok(None);
                    }

                    tx_rec.transition(
                        tx,
                        TransactionPoolStage::LocalPrepared,
                        tx_rec.transaction.evidence.all_shards_complete(),
                    )?;
                },
                Command::Accept(t) => {
                    // Happy path: We've validated all the QCs and therefore are convinced that everyone also received
                    // LocalPrepare. We then propose new blocks until we have a 3-chain
                    if !tx_rec.stage().is_local_prepared() {
                        warn!(
                            target: LOG_TARGET,
                            "❌ Stage disagreement for block {}. Leader proposed Accept, local stage {}",
                            block.id(),
                            tx_rec.stage()
                        );
                        return Ok(None);
                    }
                    if tx_rec.final_decision() != t.decision {
                        warn!(
                            target: LOG_TARGET,
                            "❌ Accept decision disagreement for block {}. Leader proposed {}, we decided {}",
                            block.id(),
                            t.decision,
                            tx_rec.final_decision()
                        );
                        return Ok(None);
                    }

                    if !tx_rec.transaction.evidence.all_shards_complete() {
                        warn!(
                            target: LOG_TARGET,
                            "❌ Accept evidence disagreement for block {}. Evidence for {} out of {} shards",
                            block.id(),
                            tx_rec.transaction.evidence.num_complete_shards(),
                            tx_rec.transaction.evidence.len(),
                        );
                        return Ok(None);
                    }
                    // If the decision was changed to Abort, which can only happen when a foreign shard decides ABORT
                    // and we decide COMMIT, we set SomePrepared, otherwise AllPrepared. These are
                    // the last stages.
                    if tx_rec.pending_decision().map(|d| d.is_abort()).unwrap_or(false) {
                        tx_rec.transition(tx, TransactionPoolStage::SomePrepared, false)?;
                    } else {
                        tx_rec.transition(tx, TransactionPoolStage::AllPrepared, false)?;
                    }
                },
            }
        }

        info!(target: LOG_TARGET, "✅ Accepting block {}", block.id());
        Ok(Some(QuorumDecision::Accept))
    }

    fn lock_inputs(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        transaction: &Transaction,
        local_committee_shard: &CommitteeShard,
    ) -> Result<bool, HotStuffError> {
        let state = SubstateRecord::try_lock_many(
            tx,
            transaction.id(),
            local_committee_shard.filter(transaction.inputs().iter().chain(transaction.filled_inputs())),
            SubstateLockFlag::Write,
        )?;
        if !state.is_acquired() {
            return Ok(false);
        }
        let state = SubstateRecord::try_lock_many(
            tx,
            transaction.id(),
            local_committee_shard.filter(transaction.input_refs()),
            SubstateLockFlag::Read,
        )?;

        if !state.is_acquired() {
            return Ok(false);
        }

        Ok(true)
    }

    fn unlock_inputs(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        transaction: &Transaction,
        local_committee_shard: &CommitteeShard,
    ) -> Result<(), HotStuffError> {
        SubstateRecord::try_unlock_many(
            tx,
            transaction.id(),
            local_committee_shard.filter(transaction.inputs().iter().chain(transaction.filled_inputs())),
            SubstateLockFlag::Write,
        )?;
        SubstateRecord::try_unlock_many(
            tx,
            transaction.id(),
            local_committee_shard.filter(transaction.input_refs()),
            SubstateLockFlag::Read,
        )?;
        Ok(())
    }

    async fn generate_vote_message(
        &self,
        block: &Block,
        decision: QuorumDecision,
    ) -> Result<VoteMessage, HotStuffError> {
        let merkle_proof = self
            .epoch_manager
            .get_validator_node_merkle_proof(block.epoch())
            .await?;
        let vn = self
            .epoch_manager
            .get_validator_node(block.epoch(), &self.validator_addr)
            .await?;
        let leaf_hash = vn.node_hash();

        let signature = self.vote_signing_service.sign_vote(&leaf_hash, block.id(), &decision);

        Ok(VoteMessage {
            epoch: block.epoch(),
            block_id: *block.id(),
            decision,
            signature,
            merkle_proof,
        })
    }

    fn update_nodes(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        block: &Block,
        local_committee_shard: &CommitteeShard,
    ) -> Result<(), HotStuffError> {
        update_high_qc(tx, block.justify())?;

        // b'' <- b*.justify.node
        let Some(commit_node) = block.justify().get_block(tx.deref_mut()).optional()? else {
            return Ok(());
        };

        // b' <- b''.justify.node
        let Some(precommit_node) = commit_node.justify().get_block(tx.deref_mut()).optional()? else {
            return Ok(());
        };

        let locked_block = LockedBlock::get(tx.deref_mut(), block.epoch())?;
        if precommit_node.height() > locked_block.height {
            debug!(target: LOG_TARGET, "LOCKED NODE SET: {} {}", precommit_node.height(), precommit_node.id());
            // precommit_node is at COMMIT phase
            precommit_node.as_locked().set(tx)?;
        }

        // b <- b'.justify.node
        let prepare_node = precommit_node.justify().block_id();
        if commit_node.parent() == precommit_node.id() && precommit_node.parent() == prepare_node {
            debug!(
                target: LOG_TARGET,
                "✅ Node {} {} forms a 3-chain b'' = {}, b' = {}, b = {}",
                block.height(),
                block.id(),
                commit_node.id(),
                precommit_node.id(),
                prepare_node,
            );

            let last_executed = LastExecuted::get(tx.deref_mut(), block.epoch())?;
            self.on_commit(tx, &last_executed, block, local_committee_shard)?;
            block.as_last_executed().set(tx)?;
        } else {
            debug!(
                target: LOG_TARGET,
                "Node {} {} DOES NOT form a 3-chain b'' = {}, b' = {}, b = {}, b* = {}",
                block.height(),
                block.id(),
                commit_node.id(),
                precommit_node.id(),
                prepare_node,
                block.id()
            );
        }

        Ok(())
    }

    fn on_commit(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        last_executed: &LastExecuted,
        block: &Block,
        local_committee_shard: &CommitteeShard,
    ) -> Result<(), HotStuffError> {
        if last_executed.height < block.height() {
            let parent = block.get_parent(tx.deref_mut())?;
            // Recurse to "catch up" any parent parent blocks we may not have executed
            self.on_commit(tx, last_executed, &parent, local_committee_shard)?;
            debug!(
                target: LOG_TARGET,
                "✅ COMMIT Node {} {}, last executed height = {}",
                block.height(),
                block.id(),
                last_executed.height
            );
            self.execute(tx, block, local_committee_shard)?;
            self.publish_event(HotstuffEvent::BlockCommitted { block_id: *block.id() });
        }
        Ok(())
    }

    fn publish_event(&self, event: HotstuffEvent) {
        let _ignore = self.tx_events.send(event);
    }

    fn execute(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        block: &Block,
        local_committee_shard: &CommitteeShard,
    ) -> Result<(), HotStuffError> {
        for cmd in block.commands() {
            let tx_rec = self.transaction_pool.get(tx, cmd.transaction_id())?;
            match cmd {
                Command::Prepare(_t) => {},
                Command::LocalPrepared(_t) => {
                    // TODO: Check if it's ok to unlock the inputs for ABORT at this point
                },
                Command::Accept(t) => {
                    debug!(
                        target: LOG_TARGET,
                        "Transaction {} is finalized ({})", tx_rec.transaction.id, t.decision
                    );
                    let mut executed = t.get_transaction(tx.deref_mut())?;
                    match t.decision {
                        // Commit the transaction substate changes.
                        Decision::Commit => {
                            self.state_manager
                                .commit_transaction(tx, block, &executed)
                                .map_err(|e| HotStuffError::StateManagerError(e.into()))?;

                            // We unlock just so that inputs that were not mutated are unlocked, even though those
                            // should be in input_refs
                            self.unlock_inputs(tx, executed.transaction(), local_committee_shard)?;
                        },
                        // Unlock the aborted inputs.
                        Decision::Abort => {
                            self.unlock_inputs(tx, executed.transaction(), local_committee_shard)?;
                        },
                    }

                    // We are now committing the containing an Accept so can remove the transaction from the pool
                    tx_rec.remove(tx)?;
                    executed.set_final_decision(t.decision).update(tx)?;
                },
            }
        }

        Ok(())
    }

    fn validate_local_proposed_block(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::ReadTransaction<'_>,
        from: &TConsensusSpec::Addr,
        candidate_block: &Block,
    ) -> Result<(), ProposalValidationError> {
        self.validate_proposed_block(from, candidate_block)?;

        // Check that details included in the justify match previously added blocks
        let Some(justify_block) = candidate_block.justify().get_block(tx).optional()? else {
            // TODO: This may mean that we have to catch up
            return Err(ProposalValidationError::JustifyBlockNotFound {
                proposed_by: from.to_string(),
                hash: *candidate_block.id(),
                justify_block: *candidate_block.justify().block_id(),
            });
        };

        if justify_block.height() != candidate_block.justify().block_height() {
            return Err(ProposalValidationError::JustifyBlockInvalid {
                proposed_by: from.to_string(),
                block_id: *candidate_block.id(),
                details: format!(
                    "Justify block height ({}) does not match justify block height ({})",
                    justify_block.height(),
                    candidate_block.justify().block_height()
                ),
            });
        }

        Ok(())
    }

    fn validate_proposed_block(
        &self,
        from: &TConsensusSpec::Addr,
        candidate_block: &Block,
    ) -> Result<(), ProposalValidationError> {
        if candidate_block.height() == NodeHeight::zero() || candidate_block.id().is_genesis() {
            return Err(ProposalValidationError::ProposingGenesisBlock {
                proposed_by: from.to_string(),
                hash: *candidate_block.id(),
            });
        }

        let calculated_hash = candidate_block.calculate_hash().into();
        if calculated_hash != *candidate_block.id() {
            return Err(ProposalValidationError::NodeHashMismatch {
                proposed_by: from.to_string(),
                hash: *candidate_block.id(),
                calculated_hash,
            });
        }

        // TODO: validate justify signatures
        // self.validate_qc(candidate_block.justify(), committee)?;

        Ok(())
    }

    /// if b_new .height > vheight && (b_new extends b_lock || b_new .justify.node.height > b_lock .height)
    ///
    /// If we have not previously voted on this block and the node extends the current locked node, then we vote
    fn should_vote(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::ReadTransaction<'_>,
        block: &Block,
    ) -> Result<bool, HotStuffError> {
        let Some(last_voted) = LastVoted::get(tx, block.epoch()).optional()? else {
            // Never voted, then validated.block.height() > last_voted.height (0)
            return Ok(true);
        };

        // if b_new .height > vheight And ...
        if block.height() <= last_voted.height {
            info!(
                target: LOG_TARGET,
                "❌ NOT voting on block {}, height {}. Block height is not greater than last voted height {}",
                block.id(),
                block.height(),
                last_voted.height,
            );
            return Ok(false);
        }

        let locked = LockedBlock::get(tx, block.epoch())?;
        let locked_block = locked.get_block(tx)?;

        // (b_new extends b_lock && b_new .justify.node.height > b_lock .height)
        if !is_safe_block(tx, block, &locked_block)? {
            info!(
                target: LOG_TARGET,
                "❌ NOT voting on block {}, height {}. Block does not satisfy safeNode predicate",
                block.id(),
                block.height(),
            );
            return Ok(false);
        }

        Ok(true)
    }
}

/// safeNode predicate (https://arxiv.org/pdf/1803.05069v6.pdf)
///
/// The safeNode predicate is a core ingredient of the protocol. It examines a proposal message
/// m carrying a QC justication m.justify, and determines whether m.node is safe to accept. The safety rule to accept
/// a proposal is the branch of m.node extends from the currently locked node lockedQC.node. On the other hand, the
/// liveness rule is the replica will accept m if m.justify has a higher view than the current lockedQC. The predicate
/// is true as long as either one of two rules holds.
fn is_safe_block<TTx: StateStoreReadTransaction>(
    tx: &mut TTx,
    block: &Block,
    locked_block: &Block,
) -> Result<bool, HotStuffError> {
    // Liveness
    if block.justify().block_height() <= locked_block.height() {
        debug!(
            target: LOG_TARGET,
            "❌ justify block height {} less than or equal to locked block height {}. Block does not satisfy safeNode predicate",
            block.justify().block_height(),
            locked_block.height(),
        );
        return Ok(false);
    }

    // Safety
    let extends = block.extends(tx, locked_block.id())?;
    if !extends {
        debug!(
            target: LOG_TARGET,
            "❌ Block {} does not extend locked block {}. Block does not satisfy safeNode predicate",
            block.id(),
            locked_block.id(),
        );
    }
    Ok(extends)
}
