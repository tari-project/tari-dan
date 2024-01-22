//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::BTreeSet, num::NonZeroU64, ops::DerefMut};

use indexmap::IndexMap;
use log::*;
use tari_common_types::types::PublicKey;
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
        QuorumCertificate,
        TransactionPool,
        TransactionPoolStage,
    },
    StateStore,
};
use tari_epoch_manager::EpochManagerReader;

use crate::{
    hotstuff::{common::EXHAUST_DIVISOR, error::HotStuffError, proposer},
    messages::{HotstuffMessage, ProposalMessage},
    traits::{ConsensusSpec, OutboundMessaging, ValidatorSignatureService},
};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::on_propose_locally";

pub struct OnPropose<TConsensusSpec: ConsensusSpec> {
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
        store: TConsensusSpec::StateStore,
        epoch_manager: TConsensusSpec::EpochManager,
        transaction_pool: TransactionPool<TConsensusSpec::StateStore>,
        signing_service: TConsensusSpec::SignatureService,
        outbound_messaging: TConsensusSpec::OutboundMessaging,
    ) -> Self {
        Self {
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

    fn build_next_block(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::ReadTransaction<'_>,
        epoch: Epoch,
        parent_block: &LeafBlock,
        high_qc: QuorumCertificate,
        proposed_by: PublicKey,
        local_committee_shard: &CommitteeShard,
        empty_block: bool,
    ) -> Result<Block, HotStuffError> {
        // TODO: Configure
        const TARGET_BLOCK_SIZE: usize = 1000;
        let batch = if empty_block {
            vec![]
        } else {
            self.transaction_pool.get_batch_for_next_block(tx, TARGET_BLOCK_SIZE)?
        };

        let mut total_leader_fee = 0;
        let locked_block = LockedBlock::get(tx)?;
        let pending_proposals = ForeignProposal::get_all_pending(tx, locked_block.block_id(), parent_block.block_id())?;
        let commands = ForeignProposal::get_all_new(tx)?
            .into_iter()
            .filter_map(|foreign_proposal| {
                if pending_proposals.iter().any(|pending_proposal| {
                    pending_proposal.bucket == foreign_proposal.bucket &&
                        pending_proposal.block_id == foreign_proposal.block_id
                }) {
                    None
                } else {
                    Some(Ok(Command::ForeignProposal(
                        foreign_proposal.set_mined_at(parent_block.height().saturating_add(NodeHeight(1))),
                    )))
                }
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
                    total_leader_fee += leader_fee;
                    Ok(Command::Accept(t.get_final_transaction_atom(leader_fee)))
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
            *parent_block.block_id(),
            high_qc,
            parent_block.height() + NodeHeight(1),
            epoch,
            proposed_by,
            commands,
            total_leader_fee,
            foreign_indexes,
            None,
        );

        let signature = self.signing_service.sign(next_block.id());
        next_block.set_signature(signature);

        Ok(next_block)
    }
}
