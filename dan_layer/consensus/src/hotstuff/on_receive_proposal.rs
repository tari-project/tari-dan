//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::ops::DerefMut;

use log::*;
use tari_dan_common_types::{
    committee::{Committee, CommitteeShard},
    optional::Optional,
    NodeHeight,
};
use tari_dan_storage::{
    consensus_models::{
        Block,
        Command,
        ExecutedTransaction,
        LastExecuted,
        LastVoted,
        LockedBlock,
        QuorumDecision,
        SubstateLockFlag,
        SubstateRecord,
        Transaction,
        TransactionPool,
        TransactionPoolStage,
    },
    StateStore,
    StateStoreReadTransaction,
};
use tokio::sync::{broadcast, mpsc};

use crate::{
    hotstuff::{
        common::update_high_qc,
        error::HotStuffError,
        event::HotstuffEvent,
        on_beat::OnBeat,
        ProposalValidationError,
    },
    messages::{HotstuffMessage, ProposalMessage, VoteMessage},
    traits::{ConsensusSpec, EpochManager, LeaderStrategy, StateManager, VoteSignatureService},
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
    on_beat: OnBeat,
}

impl<TConsensusSpec> OnReceiveProposalHandler<TConsensusSpec>
where
    TConsensusSpec: ConsensusSpec,
    HotStuffError: From<<TConsensusSpec::EpochManager as EpochManager>::Error>,
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
            on_beat,
        }
    }

    pub async fn handle(&self, from: TConsensusSpec::Addr, message: ProposalMessage) -> Result<(), HotStuffError> {
        let ProposalMessage { block } = message;

        let local_committee = self.epoch_manager.get_local_committee(block.epoch()).await?;
        if local_committee.contains(&from) {
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

            self.handle_foreign_proposal(from, block).await
        }
    }

    async fn handle_local_proposal(
        &self,
        from: TConsensusSpec::Addr,
        local_committee: Committee<TConsensusSpec::Addr>,
        block: Block,
    ) -> Result<(), HotStuffError> {
        let local_committee_shard = self.epoch_manager.get_local_committee_shard(block.epoch()).await?;

        // First save the block in one db transaction
        self.store.with_write_tx(|tx| {
            self.validate_local_proposed_block(&mut *tx, &from, &block)?;
            // Insert the block if it doesnt already exist
            block.justify().save(tx)?;
            block.save(tx)?;
            Ok::<_, HotStuffError>(())
        })?;

        let maybe_decision = self.store.with_write_tx(|tx| {
            let should_vote = self.should_vote(&mut *tx, &block)?;

            let mut maybe_decision = None;
            if should_vote {
                maybe_decision = self.decide_what_to_vote(tx, &block, &local_committee_shard)?;
            }

            self.update_nodes(tx, &block)?;
            Ok::<_, HotStuffError>(maybe_decision)
        })?;

        if let Some(decision) = maybe_decision {
            let vote = self.generate_vote_message(&block, decision).await?;
            debug!(
                target: LOG_TARGET,
                "🔥 Send {:?} VOTE for block {}, parent {}, height {} to {}",
                decision,
                block.id(),
                block.parent(),
                block.height(),
                from,
            );
            self.send_to_leader(&local_committee, vote).await?;
        }

        Ok(())
    }

    async fn handle_foreign_proposal(&self, from: TConsensusSpec::Addr, block: Block) -> Result<(), HotStuffError> {
        let vn_shard = self.epoch_manager.get_validator_shard(block.epoch(), &from).await?;
        let committee_shard = self.epoch_manager.get_committee_shard(block.epoch(), vn_shard).await?;
        self.validate_proposed_block(&from, &block)?;
        self.store
            .with_write_tx(|tx| self.on_receive_foreign_block(tx, &block, &committee_shard))?;

        // We could have ready transactions at this point, so if we're the leader for the next block we can propose
        self.on_beat.beat();

        Ok(())
    }

    async fn send_to_leader(
        &self,
        local_committee: &Committee<TConsensusSpec::Addr>,
        vote: VoteMessage,
    ) -> Result<(), HotStuffError> {
        let leader = self.leader_strategy.get_leader(local_committee, &vote.block_id, 0);
        self.tx_leader
            .send((leader.clone(), HotstuffMessage::Vote(vote)))
            .await
            .map_err(|_| HotStuffError::InternalChannelClosed {
                context: "tx_leader in OnReceiveProposalHandler::handle_local_proposal",
            })
    }

    fn decide_what_to_vote(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        block: &Block,
        local_committee_shard: &CommitteeShard,
    ) -> Result<Option<QuorumDecision>, HotStuffError> {
        block.set_as_last_voted(tx)?;

        for cmd in block.commands() {
            let transaction = ExecutedTransaction::get(tx.deref_mut(), cmd.transaction_id())?;
            let mut tx_rec = self.transaction_pool.get(tx, cmd.transaction_id())?;
            // TODO: we probably need to provide the all/some of the QCs referenced in local transactions as
            //       part of the proposal DanMessage so that there is no race condition between receiving the
            //       AllProposed and receiving the foreign proposals
            tx_rec.update_evidence(tx, local_committee_shard, *block.justify().id())?;

            match cmd {
                Command::Prepare(t) => {
                    if transaction.as_decision() == t.decision {
                        if transaction.as_decision().is_commit() {
                            // Lock all inputs for the transaction as part of LocalPrepare
                            self.lock_objects(tx, transaction.transaction(), local_committee_shard)?;
                        }

                        tx_rec.transition(tx, TransactionPoolStage::LocalPrepared, false)?;
                    } else {
                        // If we disagree with any local decision we abstain from voting
                        warn!(
                            target: LOG_TARGET,
                            "❌ Prepare decision disagreement for block {}. Leader proposed {}, we decided {}",
                            block.id(),
                            t.decision,
                            transaction.as_decision()
                        );
                        return Ok(None);
                    }
                },
                // TODO: Check these against what we have
                Command::LocalPrepared(t) => {
                    if transaction.as_decision() != t.decision {
                        warn!(
                            target: LOG_TARGET,
                            "❌ Prepare decision disagreement for block {}. Leader proposed {}, we decided {}",
                            block.id(),
                            t.decision,
                            transaction.as_decision()
                        );
                        return Ok(None);
                    }
                },
                Command::Accept(t) => {
                    if transaction.as_decision() != t.decision {
                        warn!(
                            target: LOG_TARGET,
                            "❌ Prepare decision disagreement for block {}. Leader proposed {}, we decided {}",
                            block.id(),
                            t.decision,
                            transaction.as_decision()
                        );
                        return Ok(None);
                    }
                },
            }
        }

        info!(target: LOG_TARGET, "✅ Accepting block {}", block.id());
        Ok(Some(QuorumDecision::Accept))
    }

    fn lock_objects(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        transaction: &Transaction,
        local_committee_shard: &CommitteeShard,
    ) -> Result<(), HotStuffError> {
        SubstateRecord::try_lock_many(
            tx,
            local_committee_shard.filter(transaction.inputs()),
            SubstateLockFlag::Write,
        )?;
        SubstateRecord::try_lock_many(
            tx,
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
        let leaf_hash = self
            .epoch_manager
            .get_validator_leaf_hash(block.epoch(), &self.validator_addr)
            .await?;

        let signature = self.vote_signing_service.sign_vote(&leaf_hash, block.id(), &decision);

        Ok(VoteMessage {
            epoch: block.epoch(),
            block_id: *block.id(),
            decision,
            signature,
            merkle_proof,
        })
    }

    fn on_receive_foreign_block(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        block: &Block,
        committee_shard: &CommitteeShard,
    ) -> Result<(), HotStuffError> {
        // Save the QCs if it doesnt exist already, we'll reference the QC in subsequent blocks
        block.justify().save(tx)?;

        // TODO(perf): n queries
        for cmd in block.commands() {
            let Some(mut tx_rec) = self.transaction_pool.get(tx, cmd.transaction_id()).optional()? else {
                continue;
            };
            tx_rec.update_evidence(tx, committee_shard, *block.justify().id())?;
        }

        Ok(())
    }

    fn update_nodes(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        block: &Block,
    ) -> Result<(), HotStuffError> {
        update_high_qc::<TConsensusSpec::StateStore>(tx, block.justify())?;

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
            debug!(target: LOG_TARGET, "LOCKED NODE SET: {}", precommit_node.id());
            // precommit_node is at COMMIT phase
            precommit_node.set_as_locked(tx)?;
        }

        // b <- b'.justify.node
        let prepare_node = precommit_node.justify().block_id();
        if commit_node.parent() == precommit_node.id() && precommit_node.parent() == prepare_node {
            debug!(
                target: LOG_TARGET,
                "✅ Node {} forms a 3-chain b'' = {}, b' = {}, b = {}",
                block.id(),
                commit_node.id(),
                precommit_node.id(),
                prepare_node,
            );

            let last_executed = LastExecuted::get(tx.deref_mut(), block.epoch())?;
            self.on_commit(tx, &last_executed, block)?;
            block.set_as_last_executed(tx)?;
        } else {
            debug!(
                target: LOG_TARGET,
                "Node DOES NOT form a 3-chain b'' = {}, b' = {}, b = {}, b* = {}",
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
    ) -> Result<(), HotStuffError> {
        if last_executed.height < block.height() {
            let parent = block.get_parent(tx.deref_mut())?;
            // Recurse to "catch up" any parent parent blocks we may not have executed
            self.on_commit(tx, last_executed, &parent)?;
            self.execute(tx, block)?;
            self.publish_event(HotstuffEvent::BlockCommitted { block_id: *block.id() });
        }
        Ok(())
    }

    fn publish_event(&self, event: HotstuffEvent) {
        let _ = self.tx_events.send(event);
    }

    fn execute(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        block: &Block,
    ) -> Result<(), HotStuffError> {
        for cmd in block.commands() {
            let tx_rec = self.transaction_pool.get(tx, cmd.transaction_id())?;
            match cmd {
                // At this point, all local replicas have voted to prepare (including us). We mark this transaction as
                // ready so that it can be proposed as LocalPrepared in the next block.
                Command::Prepare(_t) => {
                    tx_rec.transition(tx, TransactionPoolStage::LocalPrepared, true)?;
                },

                // All local replicas have voted LocalPrepared. If we have localprepared for all shards, we mark this as
                // AllPrepared and ready.
                Command::LocalPrepared(_t) => {
                    if tx_rec.transaction.evidence.all_shards_complete() {
                        tx_rec.transition(tx, TransactionPoolStage::AllPrepared, true)?;
                    }
                },
                Command::Accept(t) => {
                    tx_rec.remove(tx)?;
                    if t.decision.is_commit() {
                        let executed_tx = ExecutedTransaction::get(tx.deref_mut(), &t.id)?;
                        self.state_manager
                            .commit_transaction(tx, &executed_tx)
                            .map_err(|e| HotStuffError::StateManagerError(e.into()))?;
                    }
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
                justify_block: *candidate_block.justify().block_id()
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
            "❌ justify block height {} less than locked block height {}. Block does not satisfy safeNode predicate",
            block.justify().block_height(),
            locked_block.height(),
        );
        return Ok(false);
    }

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
