use std::{collections::BTreeSet, ops::DerefMut};

//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause
use log::*;
use tari_dan_common_types::{
    committee::{Committee, CommitteeShard},
    optional::Optional,
    NodeHeight,
};
use tari_dan_storage::{
    consensus_models::{
        Block,
        CommittedTransactionPool,
        ExecutedTransaction,
        LastExecuted,
        LastVoted,
        LockedBlock,
        NewTransactionPool,
        PledgeCollection,
        PrecommitTransactionPool,
        PrepareTransactionPool,
        QuorumCertificate,
        TransactionDecision,
    },
    StateStore,
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
};
use tokio::sync::mpsc;

use crate::{
    hotstuff::{common::update_high_qc, error::HotStuffError, ProposalValidationError},
    messages::{HotstuffMessage, ProposalMessage, QuorumDecision, QuorumRejectReason, VoteMessage},
    traits::{ConsensusSpec, EpochManager, LeaderStrategy, VoteSigningService},
};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::on_propose";

pub struct OnReceiveProposalHandler<TConsensusSpec: ConsensusSpec> {
    store: TConsensusSpec::StateStore,
    epoch_manager: TConsensusSpec::EpochManager,
    vote_signing_service: TConsensusSpec::VoteSigningService,
    leader_strategy: TConsensusSpec::LeaderStrategy,
    tx_leader: mpsc::Sender<(TConsensusSpec::Addr, HotstuffMessage)>,
}

impl<TConsensusSpec> OnReceiveProposalHandler<TConsensusSpec>
where
    TConsensusSpec: ConsensusSpec,
    HotStuffError: From<<TConsensusSpec::EpochManager as EpochManager>::Error>,
{
    pub fn new(
        store: TConsensusSpec::StateStore,
        epoch_manager: TConsensusSpec::EpochManager,
        vote_signing_service: TConsensusSpec::VoteSigningService,
        leader_strategy: TConsensusSpec::LeaderStrategy,
        tx_leader: mpsc::Sender<(TConsensusSpec::Addr, HotstuffMessage)>,
    ) -> Self {
        Self {
            store,
            epoch_manager,
            vote_signing_service,
            leader_strategy,
            tx_leader,
        }
    }

    pub async fn handle(&self, from: TConsensusSpec::Addr, message: ProposalMessage) -> Result<(), HotStuffError> {
        let ProposalMessage { block } = message;
        debug!(
            target: LOG_TARGET,
            "🔥 Receive PROPOSAL for block {}, parent {}, height {} from {}",
            block.id(),
            block.parent(),
            block.height(),
            from,
        );

        if !self.epoch_manager.is_epoch_active(block.epoch()).await? {
            return Err(HotStuffError::EpochNotActive {
                epoch: block.epoch(),
                context: format!("Ignoring PROPOSAL received from {}", from),
            });
        }

        let local_committee = self.epoch_manager.get_local_committee(block.epoch()).await?;
        if local_committee.contains(&from) {
            self.handle_local_proposal(from, local_committee, block).await
        } else {
            self.handle_foreign_proposal(from, block).await
        }
    }

    async fn handle_local_proposal(
        &self,
        from: TConsensusSpec::Addr,
        local_committee: Committee<TConsensusSpec::Addr>,
        block: Block,
    ) -> Result<(), HotStuffError> {
        // TODO: We may not have all the transactions in this block.
        //       In which case, we should request the missing transactions from the leader and pass them to the
        //       executor. Once the executor has completed, we can vote on this block.

        let local_committee_shard = self.epoch_manager.get_local_committee_shard(block.epoch()).await?;

        let maybe_vote = self.store.with_write_tx(|tx| {
            self.validate_proposed_block(&mut *tx, &from, &block)?;

            let should_vote = self.should_vote(&mut *tx, &block)?;

            let mut maybe_vote = None;
            if should_vote {
                maybe_vote = Some(self.decide_and_vote(tx, &block, local_committee_shard)?);
            }

            self.update_nodes(tx, &block)?;
            Ok::<_, HotStuffError>(maybe_vote)
        })?;

        if let Some(vote) = maybe_vote {
            let leader = self.leader_strategy.get_leader(&local_committee, &vote.block_id, 0);
            self.send_to_leader(leader, vote).await?;
        }

        Ok(())
    }

    async fn handle_foreign_proposal(&self, from: TConsensusSpec::Addr, block: Block) -> Result<(), HotStuffError> {
        self.store.with_write_tx(|tx| {
            self.validate_proposed_block(&mut *tx, &from, &block)?;
            self.on_receive_foreign_block(tx, &block)?;
            Ok(())
        })
    }

    async fn send_to_leader(&self, leader: &TConsensusSpec::Addr, vote: VoteMessage) -> Result<(), HotStuffError> {
        self.tx_leader
            .send((leader.clone(), HotstuffMessage::Vote(vote)))
            .await
            .map_err(|_| HotStuffError::InternalChannelClosed {
                context: "tx_leader in OnReceiveProposalHandler::handle_local_proposal",
            })
    }

    fn decide_and_vote(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        block: &Block,
        local_committee_shard: CommitteeShard,
    ) -> Result<VoteMessage, HotStuffError> {
        // Insert the block if it doesnt already exist
        block.save(tx)?;
        block.set_as_last_voted(tx)?;

        match self.decide_on_local_block(tx, block, local_committee_shard) {
            Ok(()) => {
                let vote = self.generate_vote_message(block, QuorumDecision::Accept);
                info!(target: LOG_TARGET, "✅ Accepting block {}", block.id());
                Ok(vote)
            },
            Err(e) => {
                // TODO: implement specific failure cases
                let vote = self.generate_vote_message(
                    block,
                    QuorumDecision::Reject(QuorumRejectReason::TransactionPoolsDisagree),
                );
                info!(target: LOG_TARGET, "❌ Rejecting block {}: {}", block.id(), e);
                Ok(vote)
            },
        }
    }

    fn generate_vote_message(&self, block: &Block, decision: QuorumDecision) -> VoteMessage {
        let signature = self.vote_signing_service.sign_vote(block.epoch(), block.id(), decision);
        VoteMessage {
            epoch: block.epoch(),
            block_id: *block.id(),
            decision,
            signature,
        }
    }

    /// Update the transaction pools by moving all transactions into the next pool/phase or error if this is not
    /// possible. For new transactions, inputs are pledged to their respective transactions.
    fn decide_on_local_block(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        block: &Block,
        local_committee_shard: CommitteeShard,
    ) -> Result<(), HotStuffError> {
        if !NewTransactionPool::all_decisions_match(tx.deref_mut(), block.prepared())? {
            return Err(HotStuffError::DecisionMismatch {
                block_id: *block.id(),
                pool: "new",
            });
        }
        let prepared = NewTransactionPool::move_specific_to_prepare(tx, block.prepared())?;
        self.create_local_pledges(tx, block, &prepared, local_committee_shard)?;

        let _check_pledges = PrepareTransactionPool::move_specific_to_precommit(tx, block.precommitted())?;
        let _check_pledges = PrecommitTransactionPool::move_specific_to_committed(tx, block.committed())?;

        Ok(())
    }

    fn create_local_pledges(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        block: &Block,
        transactions: &BTreeSet<TransactionDecision>,
        local_committee_shard: CommitteeShard,
    ) -> Result<PledgeCollection, HotStuffError> {
        if transactions.is_empty() {
            return Ok(PledgeCollection::new(*block.id(), vec![]));
        }

        // We only pledge for local shards that we're deciding want to commit
        let to_pledge = transactions
            .iter()
            .filter(|t| t.overall_decision.is_accept())
            .map(|t| &t.transaction_id);
        let involved_shards = ExecutedTransaction::get_involved_shards(tx.deref_mut(), to_pledge)?;
        let transactions_and_shards = involved_shards
            .into_iter()
            .map(|(t_id, shards)| (t_id, local_committee_shard.filter(shards).collect()))
            .collect();

        let pledges = PledgeCollection::pledge_many(tx, block.id(), transactions_and_shards)?;

        Ok(pledges)
    }

    /// Update the transaction pools by marking all transactions as ready
    fn on_receive_foreign_block<TTx: StateStoreWriteTransaction>(
        &self,
        tx: &mut TTx,
        block: &Block,
    ) -> Result<(), HotStuffError> {
        // TODO: Think we need to check local pledges here
        PrepareTransactionPool::mark_specific_ready(tx, block.precommitted())?;
        PrecommitTransactionPool::mark_specific_ready(tx, block.committed())?;
        CommittedTransactionPool::mark_specific_ready(tx, block.committed())?;

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
            self.on_commit(tx, last_executed, &parent)?;
            self.execute(tx, block)?;
        }
        Ok(())
    }

    fn execute(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        block: &Block,
    ) -> Result<(), HotStuffError> {
        CommittedTransactionPool::finalize_specific(tx, block.committed())?;
        // TODO: commit local substates
        Ok(())
    }

    fn validate_proposed_block(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::ReadTransaction<'_>,
        from: &TConsensusSpec::Addr,
        candidate_block: &Block,
        // committee: &Committee<TConsensusSpec::Addr>,
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

        // TODO: validate justify signatures
        // self.validate_qc(candidate_block.justify(), committee)?;

        Ok(())
    }

    /// if b_new .height > vheight || (b_new extends b_lock && b_new .justify.node.height > b_lock .height)
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

        // if b_new .height > vheight Or ...
        if block.height() > last_voted.height {
            return Ok(true);
        }

        let locked = LockedBlock::get(tx, block.epoch())?;
        let locked_qc = locked.get_quorum_certificate(tx)?;

        // (b_new extends b_lock && b_new .justify.node.height > b_lock .height)
        if !is_safe_block(tx, block, &locked_qc)? {
            info!(
                target: LOG_TARGET,
                "🔥 NOT voting on block {}, height {}. Block does not satisfy safeNode predicate",
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
    locked_qc: &QuorumCertificate,
) -> Result<bool, HotStuffError> {
    // Liveness
    if block.justify().block_height() > locked_qc.block_height() {
        return Ok(true);
    }

    let extends = block.extends(tx, locked_qc.block_id())?;
    Ok(extends)
}
