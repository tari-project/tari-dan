//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::BTreeSet,
    num::NonZeroU64,
    ops::{Deref, DerefMut},
};

use indexmap::IndexMap;
use log::*;
use tari_common::configuration::Network;
use tari_common_types::types::{FixedHash, PublicKey};
use tari_dan_common_types::{
    committee::{Committee, CommitteeShard},
    optional::Optional,
    Epoch,
    NodeHeight,
};
use tari_dan_storage::{
    consensus_models::{
        Block,
        Command,
        ForeignProposal,
        ForeignSendCounters,
        HighQc,
        LastProposed,
        LeafBlock,
        LockedBlock,
        PendingStateTreeDiff,
        QuorumCertificate,
        TransactionPool,
        TransactionPoolStage,
    },
    StateStore,
};
use tari_epoch_manager::EpochManagerReader;

use crate::{
    hotstuff::{
        calculate_state_merkle_diff,
        diff_to_substate_changes,
        error::HotStuffError,
        proposer,
        EXHAUST_DIVISOR,
    },
    messages::{HotstuffMessage, ProposalMessage},
    traits::{ConsensusSpec, OutboundMessaging, ValidatorSignatureService},
};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::on_propose_locally";

pub struct OnPropose<TConsensusSpec: ConsensusSpec> {
    network: Network,
    store: TConsensusSpec::StateStore,
    epoch_manager: TConsensusSpec::EpochManager,
    transaction_pool: TransactionPool<TConsensusSpec::StateStore>,
    signing_service: TConsensusSpec::SignatureService,
    outbound_messaging: TConsensusSpec::OutboundMessaging,
}

impl<TConsensusSpec> OnPropose<TConsensusSpec>
where TConsensusSpec: ConsensusSpec
{
    pub fn new(
        network: Network,
        store: TConsensusSpec::StateStore,
        epoch_manager: TConsensusSpec::EpochManager,
        transaction_pool: TransactionPool<TConsensusSpec::StateStore>,
        signing_service: TConsensusSpec::SignatureService,
        outbound_messaging: TConsensusSpec::OutboundMessaging,
    ) -> Self {
        Self {
            network,
            store,
            epoch_manager,
            transaction_pool,
            signing_service,
            outbound_messaging,
        }
    }

    pub async fn handle(
        &mut self,
        epoch: Epoch,
        local_committee: &Committee<TConsensusSpec::Addr>,
        leaf_block: LeafBlock,
        is_newview_propose: bool,
    ) -> Result<(), HotStuffError> {
        if let Some(last_proposed) = self.store.with_read_tx(|tx| LastProposed::get(tx)).optional()? {
            if last_proposed.height > leaf_block.height {
                // is_newview_propose means that a NEWVIEW has reached quorum and nodes are expecting us to propose.
                // Re-broadcast the previous proposal
                if is_newview_propose {
                    if let Some(next_block) = self.store.with_read_tx(|tx| last_proposed.get_block(tx)).optional()? {
                        info!(
                            target: LOG_TARGET,
                            "🌿 RE-BROADCASTING locally block {}({}) to {} validators. {} command(s), justify: {} ({}), parent: {}",
                            next_block.id(),
                            next_block.height(),
                            local_committee.len(),
                            next_block.commands().len(),
                            next_block.justify().block_id(),
                            next_block.justify().block_height(),
                            next_block.parent(),
                        );
                        self.broadcast_proposal_locally(next_block, local_committee).await?;
                        return Ok(());
                    }
                }

                info!(
                    target: LOG_TARGET,
                    "⤵️ SKIPPING propose for leaf {} because we already proposed block {}",
                    leaf_block,
                    last_proposed,
                );

                return Ok(());
            }
        }

        let validator = self.epoch_manager.get_our_validator_node(epoch).await?;
        let local_committee_shard = self.epoch_manager.get_local_committee_shard(epoch).await?;
        let (_, current_base_layer_block_hash) = self.epoch_manager.current_base_layer_block_info().await?;

        let next_block = self.store.with_write_tx(|tx| {
            let high_qc = HighQc::get(tx.deref_mut())?;
            let high_qc = high_qc.get_quorum_certificate(tx.deref_mut())?;
            let next_block = self.build_next_block(
                tx,
                epoch,
                &leaf_block,
                high_qc,
                validator.public_key,
                &local_committee_shard,
                // TODO: This just avoids issues with proposed transactions causing leader failures. Not sure if this
                //       is a good idea.
                is_newview_propose,
                current_base_layer_block_hash,
            )?;

            next_block.as_last_proposed().set(tx)?;
            Ok::<_, HotStuffError>(next_block)
        })?;

        info!(
            target: LOG_TARGET,
            "🌿 PROPOSING locally new block {} to {} validators. justify: {} ({}), parent: {}",
            next_block,
            local_committee.len(),
            next_block.justify().block_id(),
            next_block.justify().block_height(),
            next_block.parent()
        );

        self.broadcast_proposal_locally(next_block, local_committee).await?;

        Ok(())
    }

    pub async fn broadcast_proposal_locally(
        &mut self,
        next_block: Block,
        local_committee: &Committee<TConsensusSpec::Addr>,
    ) -> Result<(), HotStuffError> {
        info!(
            target: LOG_TARGET,
            "🌿 Broadcasting locally proposal {} to {} local committees",
            next_block,
            local_committee.len(),
        );

        // Broadcast to local and foreign committees
        self.outbound_messaging
            .multicast(
                local_committee.iter().map(|(addr, _)| addr),
                HotstuffMessage::Proposal(ProposalMessage {
                    block: next_block.clone(),
                }),
            )
            .await?;

        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn build_next_block(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::ReadTransaction<'_>,
        epoch: Epoch,
        parent_block: &LeafBlock,
        high_qc: QuorumCertificate,
        proposed_by: PublicKey,
        local_committee_shard: &CommitteeShard,
        empty_block: bool,
        current_base_layer_block_hash: FixedHash,
    ) -> Result<Block, HotStuffError> {
        // TODO: Configure
        const TARGET_BLOCK_SIZE: usize = 1000;
        let batch = if empty_block {
            vec![]
        } else {
            self.transaction_pool.get_batch_for_next_block(tx, TARGET_BLOCK_SIZE)?
        };
        let current_version = high_qc.block_height().as_u64(); // parent_block.height.as_u64();
        let next_height = parent_block.height() + NodeHeight(1);

        let mut total_leader_fee = 0;
        let mut substate_changes = vec![];
        let locked_block = LockedBlock::get(tx)?;
        let pending_proposals = ForeignProposal::get_all_pending(tx, locked_block.block_id(), parent_block.block_id())?;
        let commands = ForeignProposal::get_all_new(tx)?
            .into_iter()
            .filter(|foreign_proposal| {
                // If the foreign proposal is already pending, don't propose it again
                !pending_proposals.iter().any(|pending_proposal| {
                    pending_proposal.bucket == foreign_proposal.bucket &&
                        pending_proposal.block_id == foreign_proposal.block_id
                })
            })
            .map(|mut foreign_proposal| {
                foreign_proposal.set_proposed_height(parent_block.height().saturating_add(NodeHeight(1)));
                Ok(Command::ForeignProposal(foreign_proposal))
            })
            .chain(batch.into_iter().map(|t| match t.current_stage() {
                // If the transaction is New, propose to Prepare it
                TransactionPoolStage::New => Ok(Command::Prepare(t.get_local_transaction_atom())),
                // The transaction is Prepared, this stage is only _ready_ once we know that all local nodes
                // accepted Prepared so we propose LocalPrepared
                TransactionPoolStage::Prepared => Ok(Command::LocalPrepared(t.get_local_transaction_atom())),
                // The transaction is LocalPrepared, meaning that we know that all foreign and local nodes have
                // prepared. We can now propose to Accept it. We also propose the decision change which everyone
                // should agree with if they received the same foreign LocalPrepare.
                TransactionPoolStage::LocalPrepared => {
                    let involved = local_committee_shard.count_distinct_shards(t.transaction().evidence.shards_iter());
                    let involved = NonZeroU64::new(involved as u64).ok_or_else(|| {
                        HotStuffError::InvariantError(format!(
                            "Number of involved shards is zero for transaction {}",
                            t.transaction_id(),
                        ))
                    })?;
                    let leader_fee = t.calculate_leader_fee(involved, EXHAUST_DIVISOR);
                    total_leader_fee += leader_fee.fee();
                    let tx_atom = t.get_final_transaction_atom(leader_fee);
                    if tx_atom.decision.is_commit() {
                        let transaction = t.get_transaction(tx)?;
                        let result = transaction.result().ok_or_else(|| {
                            HotStuffError::InvariantError(format!(
                                "Transaction {} is committed but has no result when proposing",
                                t.transaction_id(),
                            ))
                        })?;

                        let diff = result.finalize.result.accept().ok_or_else(|| {
                            HotStuffError::InvariantError(format!(
                                "Transaction {} has COMMIT decision but execution failed when proposing",
                                t.transaction_id(),
                            ))
                        })?;
                        substate_changes.extend(diff_to_substate_changes(diff));
                    }
                    Ok(Command::Accept(tx_atom))
                },
                // Not reachable as there is nothing to propose for these stages. To confirm that all local nodes
                // agreed with the Accept, more (possibly empty) blocks with QCs will be
                // proposed and accepted, otherwise the Accept block will not be committed.
                TransactionPoolStage::AllPrepared | TransactionPoolStage::SomePrepared => {
                    unreachable!(
                        "It is invalid for TransactionPoolStage::{} to be ready to propose",
                        t.current_stage()
                    )
                },
            }))
            .collect::<Result<BTreeSet<_>, HotStuffError>>()?;

        debug!(
            target: LOG_TARGET,
            "command(s) for next block: [{}]",
            commands.iter().map(|c| c.to_string()).collect::<Vec<_>>().join(",")
        );

        let pending = PendingStateTreeDiff::get_all_up_to_commit_block(tx, high_qc.block_id())?;

        let (state_root, _) = calculate_state_merkle_diff(
            tx.deref(),
            current_version,
            next_height.as_u64(),
            pending,
            substate_changes,
        )?;

        let non_local_buckets = proposer::get_non_local_shards_from_commands(
            tx,
            &commands,
            local_committee_shard.num_committees(),
            local_committee_shard.shard(),
        )?;

        let foreign_counters = ForeignSendCounters::get_or_default(tx, parent_block.block_id())?;
        let mut foreign_indexes = non_local_buckets
            .iter()
            .map(|bucket| (*bucket, foreign_counters.get_count(*bucket) + 1))
            .collect::<IndexMap<_, _>>();

        // Ensure that foreign indexes are canonically ordered
        foreign_indexes.sort_keys();

        let mut next_block = Block::new(
            self.network,
            *parent_block.block_id(),
            high_qc,
            next_height,
            epoch,
            proposed_by,
            commands,
            state_root,
            total_leader_fee,
            foreign_indexes,
            None,
            current_base_layer_block_hash,
        );

        let signature = self.signing_service.sign(next_block.id());
        next_block.set_signature(signature);

        Ok(next_block)
    }
}
