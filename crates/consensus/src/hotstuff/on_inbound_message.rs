//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::{BTreeMap, VecDeque};

use log::*;
use tari_consensus_types::{ProposalCertificate, Vote};
use tari_epoch_manager::EpochManagerReader;
use tari_ootle_common_types::{Epoch, NodeHeight, optional::Optional};
use tari_ootle_transaction::Network;

use crate::{
    hotstuff::error::HotStuffError,
    messages::HotstuffMessage,
    traits::{ConsensusSpec, InboundMessaging, hooks::ConsensusHooks},
    validations::check_quorum_certificate_signatures,
};

const LOG_TARGET: &str = "tari::ootle::consensus::hotstuff::inbound_messages";

type IncomingMessageResult<TAddr> = Result<Option<(TAddr, HotstuffMessage)>, HotStuffError>;

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
        current_epoch: Epoch,
        current_height: NodeHeight,
        has_processed_first_block: bool,
    ) -> Option<Result<(TConsensusSpec::Addr, HotstuffMessage), HotStuffError>> {
        // Then incoming messages for the current epoch/height
        let result = self
            .message_buffer
            .next(current_epoch, current_height, has_processed_first_block)
            .await;
        match result {
            Ok(Some((from, msg))) => {
                self.hooks.on_message_received(&msg);
                Some(Ok((from, msg)))
            },
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

type EpochAndHeight = (Epoch, NodeHeight);
pub struct MessageBuffer<TConsensusSpec: ConsensusSpec> {
    network: Network,
    buffer: BTreeMap<EpochAndHeight, VecDeque<(TConsensusSpec::Addr, HotstuffMessage)>>,
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
            buffer: BTreeMap::new(),
            inbound_messaging,
            epoch_manager,
            signer_service,
        }
    }

    pub async fn next(
        &mut self,
        current_epoch: Epoch,
        current_height: NodeHeight,
        has_processed_first_block: bool,
    ) -> IncomingMessageResult<TConsensusSpec::Addr> {
        let next_height = current_height + NodeHeight(1);
        // Clear buffer with lower (epoch, heights)
        let before_len = self.buffer.len();
        self.buffer = self.buffer.split_off(&(current_epoch, next_height));
        let after_len = self.buffer.len();

        debug!(
            target: LOG_TARGET,
            "Next message for current view {}/{} (has_processed_first_block={}, discard={})",
            current_epoch,
            current_height,
            has_processed_first_block,
            before_len - after_len
        );
        if has_processed_first_block {
            // Drain all buffered messages for the current view
            if let Some(buffer) = self.buffer.get_mut(&(current_epoch, next_height)) &&
                let Some(msg_tuple) = buffer.pop_front()
            {
                return Ok(Some(msg_tuple));
            }
        }

        while let Some(result) = self.inbound_messaging.next_message().await {
            let (from, msg) = result?;

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
                    return Ok(Some((from, msg)));
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
                    self.push_to_buffer(epoch, height, from, msg);
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
    }

    /// Returns `Some(reason)` if `msg` carries a 2f+1-signed QC for an epoch strictly ahead of
    /// `current_epoch`, validated against that epoch's committee. The presence of such a QC is
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

    fn push_to_buffer(&mut self, epoch: Epoch, height: NodeHeight, from: TConsensusSpec::Addr, msg: HotstuffMessage) {
        const MAX_BUFFERED_MESSAGES_PER_VIEW: usize = 1000;
        const MAX_BUFFERED_VIEWS: usize = 100_000;
        if self.buffer.len() >= MAX_BUFFERED_VIEWS {
            warn!(
                target: LOG_TARGET,
                "🗑️ Discarding message {} for view {}/{} as buffer view limit of {} reached",
                msg,
                epoch,
                height,
                MAX_BUFFERED_VIEWS
            );
            return;
        }

        let messages_mut = self.buffer.entry((epoch, height)).or_default();
        if messages_mut.len() > MAX_BUFFERED_MESSAGES_PER_VIEW {
            warn!(
                target: LOG_TARGET,
                "🗑️ Discarding message {} for view {}/{} as buffer limit of {} reached",
                msg,
                epoch,
                height,
                MAX_BUFFERED_MESSAGES_PER_VIEW
            );
            return;
        }
        messages_mut.push_back((from, msg));
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
                    let msg_height = msg
                        .block
                        .timeout_certificate()
                        .map(|_| pc_height + NodeHeight(1))
                        .unwrap_or(pc_height);

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
            // let Some(height) = msg.max_height().checked_add(NodeHeight(1)) else {
            //     warn!(
            //         target: LOG_TARGET,
            //         "❗️ NewView message {} has invalid max height {}. Discarding.", msg, msg.max_height()
            //     );
            //     return MessageRelativeView::Discard;
            // };

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
