//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{fmt::Display, hash::Hash};

use log::*;
use tari_consensus_types::{ProposalCertificate, Vote};
use tari_epoch_manager::EpochManagerReader;
use tari_ootle_common_types::{Epoch, NodeHeight, ShardGroup, committee::Committee, optional::Optional};
use tari_ootle_transaction::Network;

use crate::{
    hotstuff::{
        epoch_state::EpochState,
        error::HotStuffError,
        view_buffer::{View, ViewBuffer},
    },
    messages::HotstuffMessage,
    traits::{ConsensusSpec, InboundMessaging, hooks::ConsensusHooks},
    validations::check_quorum_certificate_signatures,
};

const LOG_TARGET: &str = "tari::ootle::consensus::hotstuff::inbound_messages";

type IncomingMessageResult<TAddr> = Result<Option<InboundMessage<TAddr>>, HotStuffError>;

/// What the message buffer hands to the consensus worker.
pub enum InboundMessage<TAddr> {
    /// A message for the current view.
    Ready { from: TAddr, message: HotstuffMessage },
    /// A committee member proposed on a certified block more than one view ahead of this node, in its epoch. The
    /// blocks in between are missing here and no buffered proposal supplies them, so the node catches up now
    /// instead of waiting out leader timeouts. The certificate's quorum signatures have been verified.
    ProposalAhead { from: TAddr, justify: ProposalCertificate },
}

pub struct OnInboundMessage<TConsensusSpec: ConsensusSpec> {
    message_buffer: MessageBuffer<TConsensusSpec>,
    hooks: TConsensusSpec::Hooks,
}

impl<TConsensusSpec: ConsensusSpec> OnInboundMessage<TConsensusSpec> {
    pub fn new(
        network: Network,
        inbound_messaging: TConsensusSpec::InboundMessaging,
        epoch_manager: TConsensusSpec::EpochManager,
        signer_service: TConsensusSpec::SignerService,
        hooks: TConsensusSpec::Hooks,
    ) -> Self {
        Self {
            message_buffer: MessageBuffer::new(network, inbound_messaging, epoch_manager, signer_service),
            hooks,
        }
    }

    /// Returns the next message that is ready for consensus. The future returned from this function is cancel safe, and
    /// can be used with tokio::select! macro.
    pub async fn next_message(
        &mut self,
        epoch_state: &EpochState<TConsensusSpec::Addr>,
        current_height: NodeHeight,
        has_processed_first_block: bool,
    ) -> Option<Result<InboundMessage<TConsensusSpec::Addr>, HotStuffError>> {
        // Then incoming messages for the current epoch/height
        let result = self
            .message_buffer
            .next(epoch_state, current_height, has_processed_first_block)
            .await;
        match result {
            Ok(Some(InboundMessage::Ready { from, message })) => {
                self.hooks.on_message_received(&message);
                Some(Ok(InboundMessage::Ready { from, message }))
            },
            Ok(Some(ahead @ InboundMessage::ProposalAhead { .. })) => Some(Ok(ahead)),
            Ok(None) => {
                // Inbound messages terminated
                None
            },
            Err(err) => Some(Err(err)),
        }
    }

    /// Discards all buffered messages including ones queued up for processing and returns when complete.
    pub async fn discard(&mut self) {
        self.message_buffer.discard().await;
    }

    pub fn clear_buffer(&mut self) {
        self.message_buffer.clear_buffer();
    }
}

/// Size budget for messages held for views we have not reached yet. Half the default byte budget of the
/// inbound queue these messages arrive on (`max_consensus_messaging_queue_bytes`), whose reservation is
/// released once a message is handed to consensus.
const MAX_BUFFERED_BYTES: usize = 64 * 1024 * 1024;

/// Count budget for the same buffer, guarding the per-message overhead that the size budget misses. A
/// committee proposing a few views ahead of a lagging local view needs a handful of entries; this is generous
/// against that and still small against the size budget.
const MAX_BUFFERED_MESSAGES: usize = 2_000;

/// What a message with an unbounded payload — commands, transactions, pledges — is charged against the size
/// budget. The decoded size is not measurable here, so every such message is charged the largest it could have
/// arrived as: the direct messaging protocol accepts at most 4 MiB on the wire.
const UNBOUNDED_MESSAGE_SIZE: usize = 4 * 1024 * 1024;

/// What a message whose decoded payload is bounded by its own shape is charged, whatever the wire carried. Of
/// these, `Vote` — one vote and one signature — and `MissingTransactionsRequest`, capped at
/// [`MAX_REQUESTED_TRANSACTIONS`](crate::messages::MAX_REQUESTED_TRANSACTIONS) ids as it is decoded, are the
/// ones that reach the buffer. The allowance covers both with room to spare.
const FIXED_MESSAGE_SIZE: usize = 64 * 1024;

/// How many views ahead of our own, within our epoch, a message may name and still be buffered. Heights are
/// not comparable across epochs, so a message for the next epoch is always admitted; within our epoch a sender
/// picks the height, and a node lagging the committee by more than this window reaches those views by
/// importing blocks, not from a buffer.
const MAX_VIEW_LOOKAHEAD: NodeHeight = NodeHeight(20);

/// What each sender may hold of [`MAX_BUFFERED_BYTES`] at most: an even share across a committee of
/// `committee_size`, and never less than one message of the largest charge.
fn max_buffered_bytes_per_sender(committee_size: usize) -> usize {
    (MAX_BUFFERED_BYTES / committee_size.max(1)).max(UNBOUNDED_MESSAGE_SIZE)
}

pub struct MessageBuffer<TConsensusSpec: ConsensusSpec> {
    network: Network,
    buffer: ViewBuffer<TConsensusSpec::Addr, HotstuffMessage>,
    /// The highest justify certificate already reported as [`InboundMessage::ProposalAhead`], so that each
    /// certificate's signatures are checked and reported at most once.
    highest_reported_ahead: Option<View>,
    inbound_messaging: TConsensusSpec::InboundMessaging,
    epoch_manager: TConsensusSpec::EpochManager,
    signer_service: TConsensusSpec::SignerService,
}

impl<TConsensusSpec: ConsensusSpec> MessageBuffer<TConsensusSpec> {
    pub fn new(
        network: Network,
        inbound_messaging: TConsensusSpec::InboundMessaging,
        epoch_manager: TConsensusSpec::EpochManager,
        signer_service: TConsensusSpec::SignerService,
    ) -> Self {
        Self {
            network,
            buffer: ViewBuffer::new(MAX_BUFFERED_MESSAGES, MAX_BUFFERED_BYTES),
            highest_reported_ahead: None,
            inbound_messaging,
            epoch_manager,
            signer_service,
        }
    }

    pub async fn next(
        &mut self,
        epoch_state: &EpochState<TConsensusSpec::Addr>,
        current_height: NodeHeight,
        has_processed_first_block: bool,
    ) -> IncomingMessageResult<TConsensusSpec::Addr> {
        let current_epoch = epoch_state.epoch();
        let next_view = View::new(current_epoch, current_height + NodeHeight(1));
        // Clear buffer with lower (epoch, heights)
        let num_discarded = self.buffer.discard_before(next_view);

        debug!(
            target: LOG_TARGET,
            "Next message for current view {}/{} (has_processed_first_block={}, discard={}, buffered={})",
            current_epoch,
            current_height,
            has_processed_first_block,
            num_discarded,
            self.buffer.len()
        );
        if has_processed_first_block {
            // Drain all buffered messages for the current view
            if let Some((from, message)) = self.buffer.pop_front(&next_view) {
                return Ok(Some(InboundMessage::Ready { from, message }));
            }
        }

        while let Some(result) = self.inbound_messaging.next_message().await {
            // A malformed message is the sender's fault and says nothing about local state, so it is dropped and
            // the worker carries on.
            let (from, msg) = match result {
                Ok(msg) => msg,
                Err(err) => {
                    warn!(target: LOG_TARGET, "🗑️ Discard malformed inbound message: {err}");
                    continue;
                },
            };

            // Probe BEFORE the discard gate so a validator that has fallen many epochs behind
            // (e.g., long network partition) can still escalate to state sync when *any*
            // future-epoch message it receives carries a QC that validates against an
            // oracle-observed committee. The probe is the only safe trigger for cross-epoch
            // recovery — block-level catch-up cannot rescue a node from cross-epoch lag.
            if msg.epoch() > current_epoch &&
                let Some(reason) = self.probe_future_epoch_qc(&msg, current_epoch).await?
            {
                return Err(HotStuffError::NeedsSync { reason });
            }

            // If the message is for two epochs or more ahead or behind, discard it.
            if msg.epoch() > current_epoch + Epoch(1) ||
                current_epoch.checked_sub(Epoch(1)).is_some_and(|e| msg.epoch() < e)
            {
                warn!(
                    target: LOG_TARGET,
                    "🗑️ Discard non-applicable message {} for epoch {}. Current epoch is {}",
                    msg,
                    msg.epoch(),
                    current_epoch
                );
                continue;
            }

            match msg_relative_view(&msg, current_epoch, current_height, has_processed_first_block) {
                MessageRelativeView::Current => {
                    return Ok(Some(InboundMessage::Ready { from, message: msg }));
                },
                MessageRelativeView::Past { epoch, height } => {
                    info!(target: LOG_TARGET, "🗑️ Discard message {} is for previous view {}/{}. Current view {}/{}", msg, epoch, height, current_epoch, current_height);
                },
                MessageRelativeView::Future { epoch, height } => {
                    if msg.proposal().is_some() {
                        info!(target: LOG_TARGET, "🔮 Proposal {msg} is for future view {height} (Current view: {current_epoch}, {current_height})");
                    } else {
                        info!(target: LOG_TARGET, "🔮 Message {msg} is for future view {height} (Current view: {current_epoch}, {current_height})");
                    }
                    let justify_ahead = self.verified_justify_ahead(&from, &msg, epoch_state, current_height);
                    push_to_buffer(
                        &mut self.buffer,
                        epoch_state.local_committee(),
                        View::new(current_epoch, current_height),
                        View::new(epoch, height),
                        from.clone(),
                        msg,
                    );
                    if let Some(justify) = justify_ahead {
                        return Ok(Some(InboundMessage::ProposalAhead { from, justify }));
                    }
                },
                MessageRelativeView::Discard => {
                    warn!(target: LOG_TARGET, "🗑️ Discard non-applicable message {}. Current view {}/{}", msg, current_epoch, current_height);
                },
            }
        }

        info!(
            target: LOG_TARGET,
            "Inbound messaging has terminated. Current view: {}/{}", current_epoch, current_height
        );
        // Inbound messaging has terminated
        Ok(None)
    }

    pub async fn discard(&mut self) {
        self.clear_buffer();
        while self.inbound_messaging.next_message().await.is_some() {}
    }

    pub fn clear_buffer(&mut self) {
        self.buffer.clear();
        self.highest_reported_ahead = None;
    }

    /// Returns a proposal's justify certificate when a member of our committee sends it, it certifies a block on
    /// our own chain more than one view ahead of ours, its quorum signatures verify against our committee, and it is
    /// higher than any certificate already reported.
    fn verified_justify_ahead(
        &mut self,
        from: &TConsensusSpec::Addr,
        msg: &HotstuffMessage,
        epoch_state: &EpochState<TConsensusSpec::Addr>,
        current_height: NodeHeight,
    ) -> Option<ProposalCertificate> {
        let HotstuffMessage::Proposal(proposal) = msg else {
            return None;
        };
        let justify = proposal.block.justify();
        // Both gates run before the dedupe, so a certificate from another shard group's chain or a non-member
        // cannot raise `highest_reported_ahead` and mask reports from our own chain.
        if !is_own_chain_ahead(
            justify,
            epoch_state.epoch(),
            epoch_state.local_committee_info().shard_group(),
            current_height,
        ) || !epoch_state.local_committee().contains(from)
        {
            return None;
        }
        let view = View::new(justify.epoch(), justify.height());
        if self.highest_reported_ahead.is_some_and(|reported| view <= reported) {
            return None;
        }
        if let Err(err) = check_quorum_certificate_signatures::<TConsensusSpec>(
            self.network,
            justify.into(),
            epoch_state.local_committee(),
            &self.signer_service,
        ) {
            debug!(
                target: LOG_TARGET,
                "Ignoring proposal from {from} justified by {}/{}: signature check failed ({err})",
                justify.epoch(),
                justify.height()
            );
            return None;
        }
        self.highest_reported_ahead = Some(view);
        Some(justify.clone())
    }

    /// Returns `Some(reason)` if `msg` carries a 2f+1-signed QC for our own shard group's chain in an epoch strictly
    /// ahead of `current_epoch`, validated against that epoch's committee. The presence of such a QC is
    /// unforgeable proof that the network has run consensus past the local view — so we need to
    /// state-sync rather than continue waiting on peers who have moved on.
    ///
    /// The probe handles arbitrary epoch deltas: a validator that has fallen multiple epochs
    /// behind (e.g. long network partition) still escalates correctly the first time any peer
    /// delivers a future-epoch message whose QC validates against an oracle-observed committee.
    /// The trust gate is the oracle + committee signatures, not the epoch delta.
    ///
    /// Returns `None` when:
    /// - the message carries no embedded QC (e.g. `Vote`);
    /// - the QC's epoch is not strictly ahead of `current_epoch` (nothing to prove);
    /// - the local oracle has not yet observed the QC's epoch or assigned its committee (both surface as `NoEpochFound`
    ///   and are caught by `.optional()`) — buffer and re-probe on the next future-epoch message;
    /// - the QC certifies another shard group's chain, or this node has no shard group in the QC's epoch;
    /// - the QC is empty / justifies the zero block (no signatures to verify, so unforgeability doesn't hold — must not
    ///   promote on this);
    /// - signature verification fails (likely spam or a malicious peer trying to wedge us into sync mode — drop
    ///   silently and buffer normally).
    async fn probe_future_epoch_qc(
        &self,
        msg: &HotstuffMessage,
        current_epoch: Epoch,
    ) -> Result<Option<String>, HotStuffError> {
        let Some(qc) = extract_embedded_qc(msg) else {
            return Ok(None);
        };

        let qc_epoch = qc.epoch();
        if qc_epoch <= current_epoch || qc.justifies_zero_block() {
            return Ok(None);
        }

        // Did the local oracle observe `qc_epoch`? If not, we cannot fetch the committee or
        // verify the signatures; buffer and try again on the next incoming future-epoch
        // message (peers keep proposing in the new epoch, so this retries naturally).
        if self.epoch_manager.get_epoch_hash(qc_epoch).await.optional()?.is_none() {
            return Ok(None);
        }

        // Only our own chain's progress says we fell behind; another shard group runs its own chain at its own pace.
        // Shard groups can be reassigned across epochs, so ours is resolved at `qc_epoch`.
        let Some(local_committee_info) = self.epoch_manager.get_local_committee_info(qc_epoch).await.optional()? else {
            return Ok(None);
        };
        if qc.shard_group() != local_committee_info.shard_group() {
            debug!(
                target: LOG_TARGET,
                "Ignoring future-epoch QC for {qc_epoch} from {}: our shard group in that epoch is {}",
                qc.shard_group(),
                local_committee_info.shard_group()
            );
            return Ok(None);
        }

        // `get_committee_by_shard_group` surfaces an unassigned committee as `NoEpochFound`
        // (see `epoch_manager.rs:get_committee_for_shard_group`), so `.optional()` covers both
        // the "oracle hasn't observed the epoch" and "committee not yet assigned for this
        // shard group" cases. Returning `Ok(None)` here keeps the buffer / discard fall-through
        // intact and avoids any reliance on cached empty committees.
        let Some(committee) = self
            .epoch_manager
            .get_committee_by_shard_group(qc_epoch, qc.shard_group())
            .await
            .optional()?
        else {
            return Ok(None);
        };

        match check_quorum_certificate_signatures::<TConsensusSpec>(
            self.network,
            qc.into(),
            &committee,
            &self.signer_service,
        ) {
            Ok(()) => {
                let reason = format!(
                    "Received valid 2f+1 QC for {} ({} signatures) while consensus view is still in {}: network \
                     rolled over without us — escalating to state sync.",
                    qc_epoch,
                    qc.signatures().len(),
                    current_epoch,
                );
                warn!(target: LOG_TARGET, "🚨 {reason}");
                Ok(Some(reason))
            },
            Err(err) => {
                // Unverifiable QC — either a forgery from a current-epoch peer or a transient
                // committee mismatch. Don't escalate; drop silently and buffer the message.
                debug!(
                    target: LOG_TARGET,
                    "Ignoring future-epoch QC for {qc_epoch}: signature check failed ({err})"
                );
                Ok(None)
            },
        }
    }
}

/// Buffers `msg` from `from`, which names `view` ahead of `current_view`. Returns whether it was buffered.
///
/// A message for our epoch is buffered only from a member of our committee, the only senders whose messages
/// for a view of ours consensus can use. A message for the next epoch is bounded by the sender budget alone; it
/// is further than every view of ours, so it never displaces one.
fn push_to_buffer<TAddr: Eq + Hash + Clone + Display>(
    buffer: &mut ViewBuffer<TAddr, HotstuffMessage>,
    local_committee: &Committee<TAddr>,
    current_view: View,
    view: View,
    from: TAddr,
    msg: HotstuffMessage,
) -> bool {
    if view.epoch == current_view.epoch && !local_committee.contains(&from) {
        debug!(
            target: LOG_TARGET,
            "🗑️ Discarding message {} for view {} from {}, who is not a member of our committee",
            msg,
            view,
            from
        );
        return false;
    }

    if exceeds_view_lookahead(current_view, view) {
        debug!(
            target: LOG_TARGET,
            "🗑️ Discarding message {} for view {} as it is more than {} views ahead of our current view {}",
            msg,
            view,
            MAX_VIEW_LOOKAHEAD,
            current_view
        );
        return false;
    }

    let size = buffered_message_size(&msg);
    let max_sender_size = max_buffered_bytes_per_sender(local_committee.len());
    match buffer.insert(view, from.clone(), msg, size, max_sender_size) {
        Ok(inserted) => {
            if inserted.num_evicted > 0 {
                debug!(
                    target: LOG_TARGET,
                    "🗑️ Buffered a message for view {}, evicting {} message(s) for further views ({} of {} bytes, {} of {} messages held)",
                    view,
                    inserted.num_evicted,
                    buffer.size(),
                    buffer.max_size(),
                    buffer.len(),
                    buffer.max_items()
                );
            }
            true
        },
        Err(msg) => {
            debug!(
                target: LOG_TARGET,
                "🗑️ Discarding message {} for view {} from {}: the buffer is full of messages for views at or nearer than it ({} of {} bytes held for the sender, {} of {} bytes, {} of {} messages)",
                msg,
                view,
                from,
                buffer.sender_size(&from),
                max_sender_size,
                buffer.size(),
                buffer.max_size(),
                buffer.len(),
                buffer.max_items()
            );
            false
        },
    }
}

/// True when `justify` certifies a block on our own chain (our epoch and shard group) more than one view ahead of
/// `current_height`, so the blocks between are missing here. Other shard groups run their own chains, whose heights
/// say nothing about ours.
fn is_own_chain_ahead(
    justify: &ProposalCertificate,
    local_epoch: Epoch,
    local_shard_group: ShardGroup,
    current_height: NodeHeight,
) -> bool {
    justify.epoch() == local_epoch &&
        justify.shard_group() == local_shard_group &&
        justify.height() > current_height + NodeHeight(1)
}

/// Whether `view` is too far ahead of `current_view` to be worth holding. Views in a later epoch are always
/// worth holding: heights restart relative to that epoch's own progress, so the lookahead window says nothing
/// about them.
fn exceeds_view_lookahead(current_view: View, view: View) -> bool {
    view.epoch == current_view.epoch && view.height > current_view.height.saturating_add(MAX_VIEW_LOOKAHEAD)
}

/// What `msg` is charged against the buffer's size budget.
fn buffered_message_size(msg: &HotstuffMessage) -> usize {
    match msg {
        HotstuffMessage::Proposal(_) |
        HotstuffMessage::CatchUpSyncResponse(_) |
        HotstuffMessage::ForeignProposal(_) |
        HotstuffMessage::MissingTransactionsResponse(_) => UNBOUNDED_MESSAGE_SIZE,
        HotstuffMessage::NewView(_) |
        HotstuffMessage::Vote(_) |
        HotstuffMessage::ForeignProposalNotification(_) |
        HotstuffMessage::ForeignProposalRequest(_) |
        HotstuffMessage::MissingTransactionsRequest(_) |
        HotstuffMessage::CatchUpSyncRequest(_) => FIXED_MESSAGE_SIZE,
    }
}

/// Extracts a 2f+1 QC from a HotstuffMessage when one is embedded. Used by the future-epoch
/// probe to gate the `NeedsSync` escalation on cryptographic evidence rather than wall-clock
/// heuristics.
///
/// Only `Proposal` (via its block's `justify`) and `NewView` (via `high_pc`) carry QCs. `Vote`
/// is a single-validator signature and cannot prove anything about a 2f+1 quorum, so it must
/// not be used as evidence.
fn extract_embedded_qc(msg: &HotstuffMessage) -> Option<&ProposalCertificate> {
    match msg {
        HotstuffMessage::Proposal(p) => Some(p.block.justify()),
        HotstuffMessage::NewView(nv) => Some(&nv.high_pc),
        _ => None,
    }
}

enum MessageRelativeView {
    /// The message is for the current view, or is applicable to the current view
    Current,
    /// The message is for a past view
    Past { epoch: Epoch, height: NodeHeight },
    /// The message is for a future view
    Future { epoch: Epoch, height: NodeHeight },
    /// The message is not and will never be applicable
    Discard,
}

#[allow(clippy::too_many_lines)]
fn msg_relative_view(
    msg: &HotstuffMessage,
    current_epoch: Epoch,
    current_height: NodeHeight,
    has_processed_first_block: bool,
) -> MessageRelativeView {
    match msg {
        HotstuffMessage::Proposal(msg) => {
            let next_height = current_height + NodeHeight(1);
            let epoch = msg.block.epoch();
            let block_height = msg.block.height();
            let pc_height = msg.block.justify().height();

            if epoch < current_epoch {
                return MessageRelativeView::Past {
                    epoch: msg.block.epoch(),
                    height: pc_height,
                };
            }

            if epoch == current_epoch + Epoch(1) {
                return MessageRelativeView::Future {
                    epoch,
                    height: pc_height,
                };
            }

            if epoch > current_epoch {
                return MessageRelativeView::Discard;
            }

            if pc_height <= current_height &&
                msg.block
                    .timeout_certificate()
                    .is_some_and(|tc| tc.height() > current_height)
            {
                return MessageRelativeView::Current;
            }

            if pc_height < current_height || (pc_height == current_height && block_height <= current_height) {
                return MessageRelativeView::Past {
                    epoch,
                    height: pc_height,
                };
            }

            if has_processed_first_block {
                if pc_height > next_height {
                    let msg_height = if msg.block.timeout_certificate().is_some() {
                        let Some(height) = pc_height.checked_add(NodeHeight(1)) else {
                            warn!(
                                target: LOG_TARGET,
                                "❗️ Proposal {} has an out-of-range justify height {}. Discarding.", msg.block, pc_height
                            );
                            return MessageRelativeView::Discard;
                        };
                        height
                    } else {
                        pc_height
                    };

                    return MessageRelativeView::Future {
                        epoch,
                        height: msg_height,
                    };
                }
            } else {
                // (a) Special case, the justify height and the current height are zero, and we have not processed the
                // first block This is specifically to handle the case where we are starting from
                // genesis and immediately run a catch up. The first and second blocks both are for view
                // 1. If has_processed_first_block is false for the second block, we want to process it
                // in the future not immediately (which would happen in (c) below).
                // TODO: hacky
                if pc_height == NodeHeight(1) && block_height == NodeHeight(2) {
                    return MessageRelativeView::Future {
                        epoch,
                        height: NodeHeight(1),
                    };
                }
                if block_height > NodeHeight(1) {
                    return MessageRelativeView::Future {
                        epoch,
                        height: pc_height,
                    };
                }
            }

            MessageRelativeView::Current
        },
        HotstuffMessage::Vote(msg) => {
            let vote = &msg.vote;
            let vote_view_height = vote.height();
            if vote.epoch() == current_epoch && vote_view_height >= current_height {
                return MessageRelativeView::Current;
            }

            if vote.epoch() < current_epoch || (vote.epoch() == current_epoch && vote_view_height < current_height) {
                return MessageRelativeView::Past {
                    epoch: vote.epoch(),
                    height: vote_view_height,
                };
            }

            // Epoch >= current_epoch
            MessageRelativeView::Future {
                epoch: vote.epoch(),
                height: vote_view_height,
            }
        },
        HotstuffMessage::NewView(msg) => {
            let height = msg.max_height();
            if msg.epoch() < current_epoch || (msg.epoch() == current_epoch && height < current_height) {
                MessageRelativeView::Past {
                    epoch: msg.epoch(),
                    height,
                }
            } else {
                // All new view messages for future view heights are considered applicable and should be processed
                MessageRelativeView::Current
            }
        },
        HotstuffMessage::ForeignProposal(_) => {
            // Foreign proposals are always applicable
            MessageRelativeView::Current
        },
        HotstuffMessage::ForeignProposalNotification(_) => MessageRelativeView::Current,
        HotstuffMessage::ForeignProposalRequest(_) => MessageRelativeView::Current,
        HotstuffMessage::MissingTransactionsRequest(msg) => {
            if msg.epoch < current_epoch {
                return MessageRelativeView::Past {
                    epoch: msg.epoch,
                    height: NodeHeight::zero(),
                };
            }
            if msg.epoch > current_epoch {
                return MessageRelativeView::Future {
                    epoch: msg.epoch,
                    height: NodeHeight::zero(),
                };
            }
            MessageRelativeView::Current
        },
        HotstuffMessage::MissingTransactionsResponse(msg) => {
            if msg.epoch < current_epoch {
                return MessageRelativeView::Past {
                    epoch: msg.epoch,
                    height: NodeHeight::zero(),
                };
            }
            if msg.epoch > current_epoch {
                return MessageRelativeView::Future {
                    epoch: msg.epoch,
                    height: NodeHeight::zero(),
                };
            }
            MessageRelativeView::Current
        },
        HotstuffMessage::CatchUpSyncRequest(msg) => {
            if msg.epoch != current_epoch {
                return MessageRelativeView::Discard;
            }
            MessageRelativeView::Current
        },
        // Catch-up responses are an ordered block import, not live consensus. They are deliberately
        // exempt from the view filter: a node behind on blocks must ingest them regardless of where
        // its current view sits (above OR below the imported heights). Ordering is preserved by the
        // sender (FIFO, height order) and the request/response loop, and each imported block advances
        // the view monotonically, so they are never buffered or dropped here.
        HotstuffMessage::CatchUpSyncResponse(_) => MessageRelativeView::Current,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod is_own_chain_ahead {
        use tari_common_types::types::FixedHash;
        use tari_consensus_types::BlockId;
        use tari_sidechain::QuorumDecision;

        use super::*;

        fn local_group() -> ShardGroup {
            ShardGroup::new_checked(0, 127).unwrap()
        }

        fn justify(epoch: u64, shard_group: ShardGroup, height: u64) -> ProposalCertificate {
            ProposalCertificate::new(
                FixedHash::zero(),
                BlockId::zero(),
                NodeHeight(height),
                Epoch(epoch),
                shard_group,
                vec![],
                QuorumDecision::Accept,
            )
        }

        #[test]
        fn a_certificate_on_our_chain_two_views_ahead_is_ahead() {
            assert!(is_own_chain_ahead(
                &justify(1, local_group(), 12),
                Epoch(1),
                local_group(),
                NodeHeight(10)
            ));
        }

        #[test]
        fn a_certificate_for_the_next_view_is_not_ahead() {
            assert!(!is_own_chain_ahead(
                &justify(1, local_group(), 11),
                Epoch(1),
                local_group(),
                NodeHeight(10)
            ));
        }

        #[test]
        fn a_certificate_from_another_shard_group_is_not_ahead() {
            let other_group = ShardGroup::new_checked(128, 255).unwrap();
            assert!(!is_own_chain_ahead(
                &justify(1, other_group, 100),
                Epoch(1),
                local_group(),
                NodeHeight(10)
            ));
        }

        #[test]
        fn a_certificate_from_another_epoch_is_not_ahead() {
            assert!(!is_own_chain_ahead(
                &justify(2, local_group(), 100),
                Epoch(1),
                local_group(),
                NodeHeight(10)
            ));
        }
    }

    mod exceeds_view_lookahead {
        use super::*;

        fn view(epoch: u64, height: u64) -> View {
            View::new(Epoch(epoch), NodeHeight(height))
        }

        #[test]
        fn a_view_within_the_window_is_buffered() {
            let current = view(1, 10);
            assert!(!exceeds_view_lookahead(current, view(1, 11)));
            assert!(!exceeds_view_lookahead(
                current,
                View::new(Epoch(1), NodeHeight(10) + MAX_VIEW_LOOKAHEAD)
            ));
        }

        #[test]
        fn a_view_beyond_the_window_is_not_buffered() {
            let current = view(1, 10);
            assert!(exceeds_view_lookahead(
                current,
                View::new(Epoch(1), NodeHeight(11) + MAX_VIEW_LOOKAHEAD)
            ));
            assert!(exceeds_view_lookahead(current, view(1, u64::MAX)));
        }

        #[test]
        fn a_view_in_a_later_epoch_is_always_buffered() {
            let current = view(1, 10);
            assert!(!exceeds_view_lookahead(current, view(2, 1)));
            assert!(!exceeds_view_lookahead(current, view(2, u64::MAX)));
        }
    }

    mod push_to_buffer {
        use tari_consensus_types::BlockId;
        use tari_ootle_common_types::committee::CommitteeMember;

        use super::*;
        use crate::messages::MissingTransactionsResponse;

        const CURRENT_VIEW: View = View {
            epoch: Epoch(1),
            height: NodeHeight(10),
        };
        const NEXT_VIEW: View = View {
            epoch: Epoch(1),
            height: NodeHeight(11),
        };
        const NEXT_EPOCH_VIEW: View = View {
            epoch: Epoch(2),
            height: NodeHeight(1),
        };

        fn committee() -> Committee<String> {
            Committee::new(
                ["1", "2", "3", "4"]
                    .into_iter()
                    .map(|address| CommitteeMember {
                        address: address.to_string(),
                        ..Default::default()
                    })
                    .collect(),
            )
        }

        fn new_buffer() -> ViewBuffer<String, HotstuffMessage> {
            ViewBuffer::new(MAX_BUFFERED_MESSAGES, MAX_BUFFERED_BYTES)
        }

        /// A message charged the largest size, as a proposal is.
        fn large_message(epoch: Epoch) -> HotstuffMessage {
            HotstuffMessage::MissingTransactionsResponse(MissingTransactionsResponse {
                request_id: 0,
                epoch,
                block_id: BlockId::zero(),
                transactions: vec![],
            })
        }

        fn push(buffer: &mut ViewBuffer<String, HotstuffMessage>, view: View, from: &str) -> bool {
            push_to_buffer(
                buffer,
                &committee(),
                CURRENT_VIEW,
                view,
                from.to_string(),
                large_message(view.epoch),
            )
        }

        fn flood(buffer: &mut ViewBuffer<String, HotstuffMessage>, view: View, from: &str) {
            for _ in 0..MAX_BUFFERED_BYTES / UNBOUNDED_MESSAGE_SIZE * 2 {
                push(buffer, view, from);
            }
        }

        fn drain_senders(buffer: &mut ViewBuffer<String, HotstuffMessage>, view: View) -> Vec<String> {
            std::iter::from_fn(|| buffer.pop_front(&view).map(|(from, _)| from)).collect()
        }

        #[test]
        fn a_message_from_a_member_for_the_current_epoch_is_buffered() {
            let mut buffer = new_buffer();
            assert!(push(&mut buffer, NEXT_VIEW, "1"));
            assert_eq!(drain_senders(&mut buffer, NEXT_VIEW), ["1"]);
        }

        #[test]
        fn a_message_from_outside_the_committee_for_the_current_epoch_is_not_buffered() {
            let mut buffer = new_buffer();
            assert!(!push(&mut buffer, NEXT_VIEW, "outsider"));
            assert_eq!(buffer.len(), 0);
        }

        #[test]
        fn one_member_cannot_fill_the_buffer_for_a_view() {
            let mut buffer = new_buffer();
            flood(&mut buffer, NEXT_VIEW, "1");

            assert!(push(&mut buffer, NEXT_VIEW, "2"));
            assert!(drain_senders(&mut buffer, NEXT_VIEW).contains(&"2".to_string()));
        }

        #[test]
        fn one_sender_cannot_fill_the_buffer_for_the_next_epoch() {
            let mut buffer = new_buffer();
            flood(&mut buffer, NEXT_EPOCH_VIEW, "outsider");

            assert!(push(&mut buffer, NEXT_EPOCH_VIEW, "next-epoch-member"));
            assert!(drain_senders(&mut buffer, NEXT_EPOCH_VIEW).contains(&"next-epoch-member".to_string()));
        }
    }
}
