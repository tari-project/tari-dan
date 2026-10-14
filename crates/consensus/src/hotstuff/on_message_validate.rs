//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::{HashSet, VecDeque};

use log::*;
use tari_bor::BorError;
use tari_consensus_types::BlockId;
use tari_epoch_manager::EpochManagerReader;
use tari_ootle_common_types::{
    Epoch,
    NodeHeight,
    committee::{Committee, CommitteeInfo},
};
use tari_ootle_storage::{
    StateStore,
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    consensus_models::{Block, ForeignParkedProposal, ForeignProposal, ForeignProposalRecord, TransactionRecord},
};
use tari_ootle_transaction::{Transaction, TransactionId};
use tokio::sync::broadcast;

use super::config::HotstuffConfig;
use crate::{
    hotstuff::{
        CurrentView,
        HotstuffEvent,
        ProposalValidationError,
        epoch_state::EpochState,
        error::HotStuffError,
        on_receive_foreign_proposal::{OnReceiveForeignProposalHandler, validate_evidence_and_pledges_match},
        on_receive_new_transaction::OnReceiveNewTransaction,
    },
    messages::{
        EncodedTransaction,
        ForeignProposalMessage,
        ForeignProposalRequestMessage,
        HotstuffMessage,
        MAX_REQUESTED_TRANSACTIONS,
        MissingTransactionsRequest,
        ProposalMessage,
    },
    tracing::TraceTimer,
    traits::{ConsensusSpec, OutboundMessaging},
    validations,
};

const LOG_TARGET: &str = "tari::ootle::consensus::hotstuff::on_message_validate";

pub struct OnMessageValidate<TConsensusSpec: ConsensusSpec> {
    config: HotstuffConfig,
    store: TConsensusSpec::StateStore,
    epoch_manager: TConsensusSpec::EpochManager,
    current_view: CurrentView,
    vote_signing_service: TConsensusSpec::SignerService,
    outbound_messaging: TConsensusSpec::OutboundMessaging,
    tx_events: broadcast::WeakSender<HotstuffEvent>,
    /// Keep track of max 32 in-flight requests
    active_missing_transaction_requests: MissingTransactionRequests<TConsensusSpec::Addr>,
    current_request_id: u32,
}

impl<TConsensusSpec: ConsensusSpec> OnMessageValidate<TConsensusSpec> {
    pub fn new(
        config: HotstuffConfig,
        store: TConsensusSpec::StateStore,
        epoch_manager: TConsensusSpec::EpochManager,
        current_view: CurrentView,
        vote_signing_service: TConsensusSpec::SignerService,
        outbound_messaging: TConsensusSpec::OutboundMessaging,
        tx_events: broadcast::WeakSender<HotstuffEvent>,
    ) -> Self {
        Self {
            config,
            store,
            epoch_manager,
            current_view,
            vote_signing_service,
            outbound_messaging,
            tx_events,
            active_missing_transaction_requests: MissingTransactionRequests::new(),
            current_request_id: 0,
        }
    }

    pub async fn handle(
        &mut self,
        current_height: NodeHeight,
        epoch_state: &EpochState<TConsensusSpec::Addr>,
        from: TConsensusSpec::Addr,
        msg: HotstuffMessage,
        new_transactions: &OnReceiveNewTransaction<TConsensusSpec>,
        foreign_proposals: &mut OnReceiveForeignProposalHandler<TConsensusSpec>,
    ) -> Result<MessageValidationResult<TConsensusSpec::Addr>, HotStuffError> {
        let _timer = TraceTimer::debug(LOG_TARGET, "on_message_validate");
        match msg {
            HotstuffMessage::Proposal(msg) => {
                if !epoch_state.local_committee().contains(&from) {
                    warn!(
                        target: LOG_TARGET,
                        "❌ Received Proposal from non-committee member {}. Discarding message.",
                        from
                    );
                    return Ok(MessageValidationResult::Discard);
                }
                self.process_local_proposal(current_height, from, epoch_state, *msg, new_transactions)
                    .await
            },
            HotstuffMessage::CatchUpSyncResponse(msg) => {
                if !epoch_state.local_committee().contains(&from) {
                    warn!(
                        target: LOG_TARGET,
                        "❌ Received CatchUpSyncResponse from non-committee member {}. Discarding message.",
                        from
                    );
                    return Ok(MessageValidationResult::Discard);
                }
                self.process_catch_up_response(from, epoch_state, *msg, new_transactions)
                    .await
            },
            HotstuffMessage::ForeignProposal(msg) => {
                // Honest late replies to a request another peer already answered also land here, so this logs at debug.
                let block_id = msg.proposal.calculate_block_id();
                if !foreign_proposals.take_reply(&block_id, &from) {
                    debug!(target: LOG_TARGET, "❓Received foreign proposal {block_id} from {from}, who has no open request for it. Discarding message");
                    return Ok(MessageValidationResult::Discard);
                }
                self.process_foreign_proposal(epoch_state, from, msg).await
            },
            HotstuffMessage::MissingTransactionsResponse(msg) => {
                let Some(request) = self.active_missing_transaction_requests.take(msg.request_id, &from) else {
                    warn!(target: LOG_TARGET, "❓Received missing transactions (req_id = {}) from {} that we did not request. Discarding message", msg.request_id, from);
                    return Ok(MessageValidationResult::Discard);
                };

                match request.decode_response(
                    msg.transactions,
                    self.config.consensus_constants.max_transaction_size_bytes,
                ) {
                    Ok(transactions) => Ok(MessageValidationResult::RequestedTransactions {
                        from,
                        block_id: msg.block_id,
                        transactions,
                    }),
                    Err(err) => {
                        warn!(target: LOG_TARGET, "⚠️Peer {from} sent an invalid response to req_id = {}: {err}. Discarding message", msg.request_id);
                        Ok(MessageValidationResult::Discard)
                    },
                }
            },
            HotstuffMessage::MissingTransactionsRequest(msg) => {
                if !self
                    .is_missing_transactions_requester(epoch_state, &from, msg.epoch)
                    .await?
                {
                    return Ok(MessageValidationResult::Discard);
                }
                if msg.transactions.len() > MAX_REQUESTED_TRANSACTIONS {
                    warn!(target: LOG_TARGET, "⚠️Peer requested more than the maximum amount of transactions. Discarding message");
                    return Ok(MessageValidationResult::Discard);
                }
                Ok(MessageValidationResult::Ready {
                    from,
                    message: HotstuffMessage::MissingTransactionsRequest(msg),
                })
            },
            HotstuffMessage::ForeignProposalRequest(msg) => {
                if !self.is_foreign_proposal_requester(epoch_state, &from, &msg).await? {
                    return Ok(MessageValidationResult::Discard);
                }
                Ok(MessageValidationResult::Ready {
                    from,
                    message: HotstuffMessage::ForeignProposalRequest(msg),
                })
            },
            msg @ HotstuffMessage::NewView(_) |
            msg @ HotstuffMessage::Vote(_) |
            msg @ HotstuffMessage::CatchUpSyncRequest(_) => {
                if !epoch_state.local_committee().contains(&from) {
                    warn!(
                        target: LOG_TARGET,
                        "❌ Received {} from non-committee member {}. Discarding message.",
                        msg.as_type_str(),
                        from
                    );
                    return Ok(MessageValidationResult::Discard);
                }
                Ok(MessageValidationResult::Ready { from, message: msg })
            },
            msg => Ok(MessageValidationResult::Ready { from, message: msg }),
        }
    }

    /// Local committee members request transactions for local proposals and foreign committee members for foreign
    /// proposals, so any validator registered for the request's epoch may ask.
    async fn is_missing_transactions_requester(
        &self,
        epoch_state: &EpochState<TConsensusSpec::Addr>,
        from: &TConsensusSpec::Addr,
        epoch: Epoch,
    ) -> Result<bool, HotStuffError> {
        if epoch_state.local_committee().contains(from) {
            return Ok(true);
        }

        // The epoch is chosen by the requester, so a failed lookup rejects the request rather than the worker.
        match self
            .epoch_manager
            .get_committee_info_by_validator_address(epoch, from)
            .await
        {
            Ok(_) => Ok(true),
            Err(err) => {
                warn!(
                    target: LOG_TARGET,
                    "❌ Received MissingTransactionsRequest from {from} who is not a registered validator for epoch \
                     {epoch} ({err}). Discarding message.",
                );
                Ok(false)
            },
        }
    }

    /// A foreign proposal is only served to a registered validator that is a member of the foreign shard group it asks
    /// pledges for.
    async fn is_foreign_proposal_requester(
        &self,
        epoch_state: &EpochState<TConsensusSpec::Addr>,
        from: &TConsensusSpec::Addr,
        msg: &ForeignProposalRequestMessage,
    ) -> Result<bool, HotStuffError> {
        let ForeignProposalRequestMessage::ByBlockId {
            for_shard_group, epoch, ..
        } = msg;

        if *for_shard_group == epoch_state.local_committee_info().shard_group() {
            warn!(
                target: LOG_TARGET,
                "❌ Received ForeignProposalRequest from {from} for the local shard group {for_shard_group}. Discarding message.",
            );
            return Ok(false);
        }

        // `for_shard_group` is the requester's shard group in its own current epoch, which may be one either side of
        // ours across a boundary, and shard groups move between epochs when the committee count changes. The epoch is
        // chosen by the requester, so a failed lookup counts as no match.
        let current_epoch = epoch_state.epoch();
        let mut candidate_epochs = vec![current_epoch];
        for candidate in [
            Some(*epoch),
            Some(current_epoch + Epoch(1)),
            current_epoch.checked_sub(Epoch(1)),
        ]
        .into_iter()
        .flatten()
        {
            if !candidate_epochs.contains(&candidate) {
                candidate_epochs.push(candidate);
            }
        }
        for candidate in candidate_epochs {
            if let Ok(requester) = self
                .epoch_manager
                .get_committee_info_by_validator_address(candidate, from)
                .await &&
                requester.shard_group() == *for_shard_group
            {
                return Ok(true);
            }
        }

        warn!(
            target: LOG_TARGET,
            "❌ Received ForeignProposalRequest from {from} who is not a member of shard group {for_shard_group} in or \
             around epoch {current_epoch} (requested epoch {epoch}). Discarding message.",
        );
        Ok(false)
    }

    pub async fn request_missing_transactions(
        &mut self,
        to: TConsensusSpec::Addr,
        block_id: BlockId,
        epoch: Epoch,
        missing_txs: HashSet<TransactionId>,
    ) -> Result<(), HotStuffError> {
        let request_id = self.next_request_id();
        self.outbound_messaging
            .send(
                to.clone(),
                HotstuffMessage::MissingTransactionsRequest(MissingTransactionsRequest {
                    request_id,
                    block_id,
                    epoch,
                    transactions: missing_txs.clone(),
                }),
            )
            .await?;
        // Only a request that went out holds one of the few slots a response can be matched against.
        self.active_missing_transaction_requests
            .insert(request_id, to, missing_txs);
        Ok(())
    }

    fn next_request_id(&mut self) -> u32 {
        let req_id = self.current_request_id;
        self.current_request_id = self.current_request_id.wrapping_add(1);
        req_id
    }

    async fn process_local_proposal(
        &mut self,
        current_height: NodeHeight,
        from: TConsensusSpec::Addr,
        epoch_state: &EpochState<TConsensusSpec::Addr>,
        proposal: ProposalMessage,
        new_transactions: &OnReceiveNewTransaction<TConsensusSpec>,
    ) -> Result<MessageValidationResult<TConsensusSpec::Addr>, HotStuffError> {
        info!(
            target: LOG_TARGET,
            "📜 new unvalidated PROPOSAL message {} from {} (current height = {})",
            proposal.block,
            proposal.block.proposed_by(),
            current_height,
        );

        if proposal.block.height() < current_height {
            // Should never happen since the on_inbound_message handler filters these out
            info!(
                target: LOG_TARGET,
                "🔥 Block {} is lower than current height {}. Ignoring.",
                proposal.block,
                current_height
            );
            return Ok(MessageValidationResult::Discard);
        }

        if let Err(err) = self.check_local_proposal(&proposal.block, epoch_state) {
            return Ok(MessageValidationResult::Invalid {
                from,
                message: HotstuffMessage::new_proposal(proposal),
                err,
            });
        }

        self.handle_missing_transactions_local_block(
            from,
            epoch_state,
            proposal,
            new_transactions,
            HotstuffMessage::new_proposal,
        )
        .await
    }

    /// Validate a block delivered via catch-up sync. This is an ordered block import, so — unlike
    /// [`Self::process_local_proposal`] — it is intentionally NOT gated on the current view height:
    /// a node whose view has advanced beyond its stored blocks must still ingest the gap. Stateless
    /// validation and missing-transaction parking are identical to the live-proposal path.
    async fn process_catch_up_response(
        &mut self,
        from: TConsensusSpec::Addr,
        epoch_state: &EpochState<TConsensusSpec::Addr>,
        proposal: ProposalMessage,
        new_transactions: &OnReceiveNewTransaction<TConsensusSpec>,
    ) -> Result<MessageValidationResult<TConsensusSpec::Addr>, HotStuffError> {
        info!(
            target: LOG_TARGET,
            "🌐 new unvalidated CATCH-UP block {} from {}",
            proposal.block,
            from,
        );

        if let Err(err) = self.check_local_proposal(&proposal.block, epoch_state) {
            return Ok(MessageValidationResult::Invalid {
                from,
                message: HotstuffMessage::new_catch_up_sync_response(proposal),
                err,
            });
        }

        self.handle_missing_transactions_local_block(
            from,
            epoch_state,
            proposal,
            new_transactions,
            HotstuffMessage::new_catch_up_sync_response,
        )
        .await
    }

    pub fn update_parked_blocks<'a, I: IntoIterator<Item = &'a TransactionId> + ExactSizeIterator>(
        &self,
        current_height: NodeHeight,
        transaction_ids: I,
    ) -> Result<(Vec<ProposalMessage>, Vec<ForeignProposalMessage>), HotStuffError> {
        let _timer = TraceTimer::debug(LOG_TARGET, "update_local_parked_blocks").with_iterations(transaction_ids.len());
        self.store.with_write_tx(|tx| {
            // TODO(perf)
            let mut unparked_blocks = Vec::new();
            let mut foreign_unparked_blocks = Vec::new();
            for transaction_id in transaction_ids {
                debug!(target: LOG_TARGET, "🔍 Checking if transaction {} unparks any blocks", transaction_id);
                if let Some((unparked_block, foreign_proposals)) =
                    tx.parked_block_remove_missing_transaction(current_height + NodeHeight(1), transaction_id)?
                {
                    info!(target: LOG_TARGET, "♻️ all transactions for local block {unparked_block} are ready for consensus");

                    self.publish_event(HotstuffEvent::ParkedBlockReady {
                        block: unparked_block.as_leaf(),
                    });

                    unparked_blocks.push(ProposalMessage {
                        block: unparked_block,
                        foreign_proposals,
                    });
                } else {
                    debug!(target: LOG_TARGET, "🔍 Transaction {} did not unpark any blocks", transaction_id);
                }

                let foreign_unparked = ForeignParkedProposal::remove_by_transaction_id(tx, transaction_id)?;
                if foreign_unparked.is_empty() {
                    debug!(target: LOG_TARGET, "🔍 Transaction {} did not unpark any foreign blocks", transaction_id);
                }else{
                    info!(target: LOG_TARGET, "♻️ all transactions for {} foreign block(s) are ready for consensus", foreign_unparked.len());
                    foreign_unparked_blocks.extend(foreign_unparked.into_iter().map(Into::into));
                }
            }
            Ok((unparked_blocks, foreign_unparked_blocks))
        })
    }

    fn check_local_proposal(
        &self,
        block: &Block,
        epoch_state: &EpochState<TConsensusSpec::Addr>,
    ) -> Result<(), HotStuffError> {
        validations::check_local_proposal::<TConsensusSpec>(
            self.current_view.get_epoch(),
            block,
            epoch_state.local_committee(),
            epoch_state.local_committee_info(),
            &self.vote_signing_service,
            &self.config,
            epoch_state.epoch_hash(),
            epoch_state.exhaust_burn_rate(),
        )
    }

    fn check_foreign_proposal(
        &self,
        proposal: &ForeignProposal,
        committee: &Committee<TConsensusSpec::Addr>,
    ) -> Result<(), HotStuffError> {
        validations::check_foreign_proposal::<TConsensusSpec>(proposal, committee, &self.config)
    }

    /// Parks the block if it is waiting on transactions this node does not hold, otherwise passes it on as the
    /// message `to_message` builds.
    async fn handle_missing_transactions_local_block(
        &mut self,
        from: TConsensusSpec::Addr,
        epoch_state: &EpochState<TConsensusSpec::Addr>,
        proposal: ProposalMessage,
        new_transactions: &OnReceiveNewTransaction<TConsensusSpec>,
        to_message: fn(ProposalMessage) -> HotstuffMessage,
    ) -> Result<MessageValidationResult<TConsensusSpec::Addr>, HotStuffError> {
        let has_missing_transactions = self.store.with_write_tx(|tx| {
            self.resequence_unpooled_transactions(tx, epoch_state, &proposal, new_transactions)?;
            let missing = self.get_missing_transactions(&**tx, epoch_state.local_committee_info(), &proposal)?;
            Ok::<_, HotStuffError>(!missing.is_empty())
        })?;

        if !has_missing_transactions {
            return Ok(MessageValidationResult::Ready {
                from,
                message: to_message(proposal),
            });
        }

        if !self
            .are_embedded_foreign_proposals_parkable(epoch_state, &proposal)
            .await?
        {
            return Ok(MessageValidationResult::Discard);
        }

        let missing_tx_ids = self.store.with_write_tx(|tx| {
            self.check_for_missing_transactions(tx, epoch_state.local_committee_info(), &proposal)
        })?;

        if missing_tx_ids.is_empty() {
            return Ok(MessageValidationResult::Ready {
                from,
                message: to_message(proposal),
            });
        }

        self.publish_event(HotstuffEvent::ProposedBlockParked {
            block: proposal.block.as_leaf(),
            num_missing_txs: missing_tx_ids.len(),
            // TODO: remove
            num_awaiting_txs: 0,
        });

        Ok(MessageValidationResult::ParkedProposal {
            block_id: *proposal.block.id(),
            epoch: proposal.block.epoch(),
            missing_txs: missing_tx_ids,
        })
    }

    /// Rebuilds pool records for this block's transactions that we hold but that are absent from the
    /// transaction pool, so that a proposal is not rejected over derived state no peer can resend.
    ///
    /// Only the commands the rebuilt `New` stage can serve are offered: a later command is not made
    /// votable by a `New` record, and that record would then be proposed at a stage the committee has
    /// moved past. See [`OnReceiveNewTransaction::resequence_known_transactions`].
    fn resequence_unpooled_transactions(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        epoch_state: &EpochState<TConsensusSpec::Addr>,
        proposal: &ProposalMessage,
        new_transactions: &OnReceiveNewTransaction<TConsensusSpec>,
    ) -> Result<(), HotStuffError> {
        let ids = proposal
            .block
            .commands()
            .iter()
            .filter_map(|cmd| {
                cmd.local_only()
                    .map(|atom| atom.id)
                    .or_else(|| cmd.local_prepare().map(|atom| atom.id))
            })
            .collect::<Vec<_>>();

        if ids.is_empty() {
            return Ok(());
        }

        new_transactions.resequence_known_transactions(tx, epoch_state.epoch(), ids, epoch_state.local_committee_info())
    }

    fn get_missing_transactions<TTx: StateStoreReadTransaction>(
        &self,
        tx: &TTx,
        local_committee_info: &CommitteeInfo,
        proposal: &ProposalMessage,
    ) -> Result<HashSet<TransactionId>, HotStuffError> {
        if proposal.block.commands().is_empty() {
            debug!(
                target: LOG_TARGET,
                "✅ Block {} is empty (no missing transactions)", proposal.block
            );
            return Ok(HashSet::new());
        }
        let mut missing_tx_ids = TransactionRecord::get_missing(tx, proposal.block.all_transaction_ids())?;
        // Also park block if it has missing transactions from foreign proposals
        for proposal in &proposal.foreign_proposals {
            let foreign_missing =
                self.get_missing_transactions_for_foreign_proposal(tx, local_committee_info, proposal)?;
            missing_tx_ids.extend(foreign_missing);
        }
        Ok(missing_tx_ids)
    }

    /// The first parked copy of a block holds its slot until it is unparked, so a block is only parked if every
    /// embedded foreign proposal not already stored is authenticated by its committee and pledges the values its
    /// evidence commits to. A copy that fails is turned away, leaving the slot for the genuine block.
    async fn are_embedded_foreign_proposals_parkable(
        &self,
        epoch_state: &EpochState<TConsensusSpec::Addr>,
        proposal: &ProposalMessage,
    ) -> Result<bool, HotStuffError> {
        for foreign_proposal in &proposal.foreign_proposals {
            let block_id = foreign_proposal.calculate_block_id();
            if self
                .store
                .with_read_tx(|tx| ForeignProposalRecord::record_exists(tx, &block_id))?
            {
                continue;
            }

            let rejection = match validations::authenticate_foreign_proposal::<TConsensusSpec>(
                &self.epoch_manager,
                foreign_proposal,
                &self.config,
            )
            .await
            {
                Ok(()) => validate_evidence_and_pledges_match(
                    foreign_proposal,
                    epoch_state.local_committee_info().shard_group(),
                )
                .err()
                .filter(|err| matches!(err, ProposalValidationError::ForeignPledgesNotCommitted { .. })),
                Err(HotStuffError::ProposalValidationError(err)) => Some(err),
                Err(err) => return Err(err),
            };
            if let Some(err) = rejection {
                warn!(
                    target: LOG_TARGET,
                    "⚠️ Not parking block {}: embedded foreign proposal {} failed validation: {}",
                    proposal.block,
                    foreign_proposal,
                    err,
                );
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn check_for_missing_transactions(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        local_committee_info: &CommitteeInfo,
        proposal: &ProposalMessage,
    ) -> Result<HashSet<TransactionId>, HotStuffError> {
        let missing_tx_ids = self.get_missing_transactions(&**tx, local_committee_info, proposal)?;

        if missing_tx_ids.is_empty() {
            debug!(
                target: LOG_TARGET,
                "✅ Block {} has no missing transactions", proposal.block
            );
            return Ok(HashSet::new());
        }

        info!(
            target: LOG_TARGET,
            "⏳ Block {} has {} missing transactions", proposal.block, missing_tx_ids.len(),
        );

        tx.parked_block_insert(&proposal.block, &proposal.foreign_proposals, &missing_tx_ids)?;

        Ok(missing_tx_ids)
    }

    async fn process_foreign_proposal(
        &self,
        epoch_state: &EpochState<TConsensusSpec::Addr>,
        from: TConsensusSpec::Addr,
        msg: ForeignProposalMessage,
    ) -> Result<MessageValidationResult<TConsensusSpec::Addr>, HotStuffError> {
        info!(
            target: LOG_TARGET,
            "🧩 new unvalidated FOREIGN PROPOSAL message {} from {}",
            msg,
            from
        );

        if msg.proposal.commit_proof().commands().is_empty() {
            warn!(
                target: LOG_TARGET,
                "❌ Foreign proposal block {} is empty; therefore, it cannot involve the local shard group", msg.proposal,
            );
            let block_id = msg.proposal.calculate_block_id();
            return Ok(MessageValidationResult::Invalid {
                from,
                message: HotstuffMessage::ForeignProposal(msg),
                err: ProposalValidationError::NoTransactionsInCommittee { block_id }.into(),
            });
        }

        let committee = match validations::resolve_foreign_committee(&self.epoch_manager, &msg.proposal).await {
            Ok(Some(committee)) => committee,
            Ok(None) => return Ok(MessageValidationResult::Discard),
            Err(HotStuffError::ProposalValidationError(err)) => {
                return Ok(MessageValidationResult::Invalid {
                    from,
                    message: HotstuffMessage::ForeignProposal(msg),
                    err: err.into(),
                });
            },
            Err(err) => return Err(err),
        };

        if let Err(err) = self.check_foreign_proposal(&msg.proposal, &committee) {
            return Ok(MessageValidationResult::Invalid {
                from,
                message: HotstuffMessage::ForeignProposal(msg),
                err,
            });
        }

        self.store.with_write_tx(|tx| {
            let all_involved_transactions = msg
                .proposal
                .all_transaction_ids_in_committee(epoch_state.local_committee_info());
            // CASE: all foreign proposals must include evidence
            let num_transactions = all_involved_transactions.clone().count();
            if num_transactions == 0 {
                warn!(
                    target: LOG_TARGET,
                    "❌ Foreign Block {} has no transactions involving our committee", msg.proposal
                );
                // drop the borrow of msg.block
                drop(all_involved_transactions);
                let block_id = msg.proposal.calculate_block_id();
                return Ok(MessageValidationResult::Invalid {
                    from,
                    message: HotstuffMessage::ForeignProposal(msg),
                    err: ProposalValidationError::NoTransactionsInCommittee {
                        block_id,
                    }.into(),
                });
            }

            let missing_tx_ids = TransactionRecord::get_missing(&**tx, all_involved_transactions)?;

            if missing_tx_ids.is_empty() {
                debug!(
                    target: LOG_TARGET,
                    "✅ Foreign Block {} has no missing transactions (out of {} transaction(s) involving this shard group)", msg.proposal,
                    num_transactions
                );
                return Ok(MessageValidationResult::Ready {
                    from,
                    message: HotstuffMessage::ForeignProposal(msg),
                });
            }

            info!(
                target: LOG_TARGET,
                "⏳ Foreign Block {} has {} missing transactions", msg.proposal, missing_tx_ids.len(),
            );

            // The first parked copy of a block holds its slot until unparked, so a copy whose pledges its evidence does
            // not commit to is turned away here, leaving the slot for the genuine proposal.
            if let Err(err @ ProposalValidationError::ForeignPledgesNotCommitted { .. }) =
                validate_evidence_and_pledges_match(&msg.proposal, epoch_state.local_committee_info().shard_group())
            {
                warn!(target: LOG_TARGET, "⚠️ Discarding foreign proposal from {from}: {err}");
                return Ok(MessageValidationResult::Discard);
            }

            let parked_block = ForeignParkedProposal::from(msg);
            if parked_block.save(tx)? {
                parked_block.add_missing_transactions(tx, &missing_tx_ids)?;
            }

            Ok(MessageValidationResult::ParkedProposal {
                block_id: *parked_block.block_id(),
                epoch: parked_block.epoch(),
                missing_txs: missing_tx_ids,
            })
        })
    }

    fn get_missing_transactions_for_foreign_proposal<TTx: StateStoreReadTransaction>(
        &self,
        tx: &TTx,
        local_committee_info: &CommitteeInfo,
        proposal: &ForeignProposal,
    ) -> Result<HashSet<TransactionId>, HotStuffError> {
        let mut all_involved_transactions = proposal
            .all_transaction_ids_in_committee(local_committee_info)
            .peekable();

        if all_involved_transactions.peek().is_none() {
            return Ok(HashSet::new());
        }

        let missing = TransactionRecord::get_missing(tx, all_involved_transactions)?;

        Ok(missing)
    }

    fn publish_event(&self, event: HotstuffEvent) {
        if let Some(sender) = self.tx_events.upgrade() {
            let _ignore = sender.send(event);
        }
    }
}

#[derive(Debug)]
pub enum MessageValidationResult<TAddr> {
    Ready {
        from: TAddr,
        message: HotstuffMessage,
    },
    /// The decoded transactions of a response to a missing-transactions request this node made.
    RequestedTransactions {
        from: TAddr,
        block_id: BlockId,
        transactions: Vec<Transaction>,
    },
    ParkedProposal {
        block_id: BlockId,
        epoch: Epoch,
        missing_txs: HashSet<TransactionId>,
    },
    Discard,
    Invalid {
        from: TAddr,
        message: HotstuffMessage,
        err: HotStuffError,
    },
}

/// A missing-transaction request we sent and are still willing to accept a response for.
#[derive(Debug, Clone)]
struct PendingMissingTransactionsRequest<TAddr> {
    request_id: u32,
    /// The peer the request went to. A response only counts when it comes back from this peer.
    to: TAddr,
    transactions: HashSet<TransactionId>,
}

impl<TAddr> PendingMissingTransactionsRequest<TAddr> {
    /// Decodes the transactions a peer returned for this request, one at a time, refusing the response at the first
    /// transaction that is over `max_transaction_size_bytes`, was not requested or is returned twice. Only
    /// transactions this request names are ever held decoded, so the heap a response costs is bounded by the
    /// transactions this node asked for.
    fn decode_response(
        &self,
        transactions: Vec<EncodedTransaction>,
        max_transaction_size_bytes: usize,
    ) -> Result<Vec<Transaction>, RequestedTransactionsError> {
        if transactions.len() > self.transactions.len() {
            return Err(RequestedTransactionsError::TooMany {
                returned: transactions.len(),
                requested: self.transactions.len(),
            });
        }

        let mut returned_ids = HashSet::with_capacity(transactions.len());
        let mut decoded = Vec::with_capacity(transactions.len());
        for encoded in transactions {
            if encoded.len() > max_transaction_size_bytes {
                return Err(RequestedTransactionsError::TooLarge {
                    size: encoded.len(),
                    max: max_transaction_size_bytes,
                });
            }
            let transaction = encoded.decode().map_err(RequestedTransactionsError::Malformed)?;
            let id = transaction.calculate_id();
            if !self.transactions.contains(&id) {
                return Err(RequestedTransactionsError::Unrequested { transaction_id: id });
            }
            if !returned_ids.insert(id) {
                return Err(RequestedTransactionsError::Duplicate { transaction_id: id });
            }
            decoded.push(transaction);
        }
        Ok(decoded)
    }
}

#[derive(Debug, thiserror::Error)]
enum RequestedTransactionsError {
    #[error("returned {returned} transaction(s) but only {requested} were requested")]
    TooMany { returned: usize, requested: usize },
    #[error("returned a {size} byte transaction, over the {max} byte limit")]
    TooLarge { size: usize, max: usize },
    #[error("returned a transaction that does not decode: {0}")]
    Malformed(BorError),
    #[error("returned transaction {transaction_id}, which was not requested")]
    Unrequested { transaction_id: TransactionId },
    #[error("returned transaction {transaction_id} more than once")]
    Duplicate { transaction_id: TransactionId },
}

/// A fixed-capacity ring of in-flight missing-transaction requests. Capacity is a bound on concurrent
/// requests, not a guarantee: the oldest entry is dropped once it is full, and its response is then
/// discarded as unrequested.
#[derive(Debug, Clone)]
struct MissingTransactionRequests<TAddr> {
    requests: VecDeque<PendingMissingTransactionsRequest<TAddr>>,
}

impl<TAddr: PartialEq> MissingTransactionRequests<TAddr> {
    const CAPACITY: usize = 32;

    pub fn new() -> Self {
        Self {
            requests: VecDeque::with_capacity(Self::CAPACITY),
        }
    }

    pub fn insert(&mut self, request_id: u32, to: TAddr, transactions: HashSet<TransactionId>) {
        if self.requests.len() == Self::CAPACITY {
            self.requests.pop_front();
        }
        self.requests.push_back(PendingMissingTransactionsRequest {
            request_id,
            to,
            transactions,
        });
    }

    /// Removes and returns the request with this id that was sent to `from`, if any.
    pub fn take(&mut self, request_id: u32, from: &TAddr) -> Option<PendingMissingTransactionsRequest<TAddr>> {
        let pos = self
            .requests
            .iter()
            .position(|req| req.request_id == request_id && req.to == *from)?;
        self.requests.remove(pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(n: u8) -> HashSet<TransactionId> {
        [TransactionId::new([n; 32])].into_iter().collect()
    }

    #[test]
    fn a_response_from_a_peer_we_did_not_ask_is_not_matched() {
        let mut requests = MissingTransactionRequests::new();
        requests.insert(1, "alice", req(1));

        assert!(requests.take(1, &"bob").is_none());
        assert!(requests.take(1, &"alice").is_some());
    }

    #[test]
    fn the_same_id_to_two_peers_matches_each_peer_once() {
        let mut requests = MissingTransactionRequests::new();
        requests.insert(1, "alice", req(1));
        requests.insert(1, "bob", req(2));

        assert_eq!(requests.take(1, &"bob").unwrap().transactions, req(2));
        assert_eq!(requests.take(1, &"alice").unwrap().transactions, req(1));
        assert!(requests.take(1, &"alice").is_none());
    }

    #[test]
    fn the_oldest_request_is_evicted_once_capacity_is_reached() {
        let mut requests = MissingTransactionRequests::new();
        for i in 0..=MissingTransactionRequests::<&str>::CAPACITY {
            requests.insert(i as u32, "alice", req(0));
        }

        assert!(requests.take(0, &"alice").is_none());
        assert!(
            requests
                .take(MissingTransactionRequests::<&str>::CAPACITY as u32, &"alice")
                .is_some()
        );
    }

    const MAX_SIZE: usize = 1024 * 1024;

    fn transaction(n: u64) -> Transaction {
        Transaction::builder_localnet(Epoch(1)).build_and_seal(&tari_common_types::types::PrivateKey::from(n))
    }

    fn encoded(transaction: &Transaction) -> EncodedTransaction {
        EncodedTransaction::encode(transaction).unwrap()
    }

    fn undecodable() -> EncodedTransaction {
        EncodedTransaction::from_bytes(vec![0xff; 16])
    }

    fn request_for(transactions: &[&Transaction]) -> PendingMissingTransactionsRequest<&'static str> {
        PendingMissingTransactionsRequest {
            request_id: 1,
            to: "alice",
            transactions: transactions.iter().map(|t| t.calculate_id()).collect(),
        }
    }

    #[test]
    fn the_requested_transactions_decode() {
        let (a, b) = (transaction(1), transaction(2));
        let request = request_for(&[&a, &b]);

        let decoded = request
            .decode_response(vec![encoded(&b), encoded(&a)], MAX_SIZE)
            .unwrap();

        let ids = decoded.iter().map(|t| t.calculate_id()).collect::<Vec<_>>();
        assert_eq!(ids, vec![b.calculate_id(), a.calculate_id()]);
    }

    #[test]
    fn more_transactions_than_were_requested_are_refused_before_any_is_decoded() {
        let a = transaction(1);
        let request = request_for(&[&a]);

        let err = request
            .decode_response(vec![undecodable(), undecodable()], MAX_SIZE)
            .unwrap_err();

        assert!(matches!(err, RequestedTransactionsError::TooMany { .. }), "{err}");
    }

    #[test]
    fn an_oversized_transaction_is_refused_before_it_is_decoded() {
        let a = transaction(1);
        let request = request_for(&[&a]);

        let err = request.decode_response(vec![undecodable()], 8).unwrap_err();

        assert!(matches!(err, RequestedTransactionsError::TooLarge { .. }), "{err}");
    }

    #[test]
    fn decoding_stops_at_the_first_unrequested_transaction() {
        let (a, b, c) = (transaction(1), transaction(2), transaction(3));
        let request = request_for(&[&a, &c]);

        let err = request
            .decode_response(vec![encoded(&b), undecodable()], MAX_SIZE)
            .unwrap_err();

        assert!(
            matches!(err, RequestedTransactionsError::Unrequested { transaction_id } if transaction_id == b.calculate_id()),
            "{err}"
        );
    }

    #[test]
    fn a_transaction_returned_twice_is_refused() {
        let (a, b) = (transaction(1), transaction(2));
        let request = request_for(&[&a, &b]);

        let err = request
            .decode_response(vec![encoded(&a), encoded(&a)], MAX_SIZE)
            .unwrap_err();

        assert!(matches!(err, RequestedTransactionsError::Duplicate { .. }), "{err}");
    }
}
