//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::sync::{Arc, Mutex, PoisonError};

use tari_consensus::{
    hotstuff::{ConsensusCurrentState, ConsensusStateEvent, HotStuffError, ProposalValidationError},
    messages::HotstuffMessage,
    traits::hooks::ConsensusHooks,
};
use tari_consensus_types::BlockId;
use tari_ootle_common_types::{
    NodeHeight,
    diag_event,
    diagnostics::{DiagnosticEvent, DiagnosticLevel},
};
use tari_ootle_storage::consensus_models::{Block, NoVoteReason, TransactionPoolRecord, ValidBlock, VoteEquivocation};
use tari_ootle_transaction::TransactionId;

use crate::diagnostics::handle::DiagnosticsHandle;

/// Records the consensus events worth knowing about into the diagnostic event log. The hooks that
/// fire on the happy path (every message, every committed block, every ready transaction) stay
/// empty: only the abnormal is recorded.
#[derive(Debug, Clone)]
pub struct DiagnosticHooks {
    diagnostics: DiagnosticsHandle,
    /// The last alarm seen of each kind, shared by every clone of these hooks. Each kind gets its
    /// own slot so that a node diverging both ways records both. See
    /// [`DiagnosticHooks::is_new_stall_alarm`].
    last_stall_alarms: Arc<Mutex<LastStallAlarms>>,
}

#[derive(Debug, Default)]
struct LastStallAlarms {
    epoch_hash: Option<String>,
    protocol_version: Option<String>,
}

impl DiagnosticHooks {
    pub fn new(diagnostics: DiagnosticsHandle) -> Self {
        Self {
            diagnostics,
            last_stall_alarms: Arc::new(Mutex::new(LastStallAlarms::default())),
        }
    }

    /// Whether this stall alarm describes a different divergence from the one before it.
    ///
    /// A stalled node rejects every proposal the committee makes, so the same divergence arrives
    /// once per proposal — on a different block each time — for as long as the stall lasts.
    /// `handle_hotstuff_error` keeps one alarm per divergence for this reason, but it reports the
    /// error to the hooks before reaching that guard, so the same distinction is drawn again here.
    fn is_new_stall_alarm(&self, alarm: &StallAlarm) -> bool {
        let mut last = self.last_stall_alarms.lock().unwrap_or_else(PoisonError::into_inner);
        let slot = match alarm.kind {
            StallAlarmKind::EpochHash => &mut last.epoch_hash,
            StallAlarmKind::ProtocolVersion => &mut last.protocol_version,
        };
        if slot.as_deref() == Some(alarm.divergence.as_str()) {
            return false;
        }
        *slot = Some(alarm.divergence.clone());
        true
    }
}

impl ConsensusHooks for DiagnosticHooks {
    fn on_local_block_committed(&mut self, _block: &ValidBlock) {}

    fn on_blocks_committed(&mut self, _committed_blocks: &[Block], _finalized_transactions: &[TransactionPoolRecord]) {}

    /// Every error this fires for goes on to reach [`Self::on_error`], which records it with the
    /// full error text, so recording it here as well would write the same row twice. `on_error` also
    /// sees the validation failures that reach consensus by other paths.
    fn on_block_validation_failed<E: ToString>(&mut self, _err: &E) {}

    fn on_message_received(&mut self, _message: &HotstuffMessage) {}

    fn on_error(&mut self, err: &HotStuffError) {
        if let Some(alarm) = stall_alarm(err) &&
            !self.is_new_stall_alarm(&alarm)
        {
            return;
        }

        let (topic, level) = classify_error(err);
        self.diagnostics
            .emit(DiagnosticEvent::new(level, topic, err.to_string()).with_field("error", err));
    }

    fn on_pacemaker_height_changed(&mut self, _height: NodeHeight) {}

    fn on_leader_timeout(&mut self, new_height: NodeHeight) {
        self.diagnostics.emit(diag_event!(
            warn,
            "consensus.leader_failure",
            "The leader failed to propose. Moving to height {new_height}",
            new_height => new_height
        ));
    }

    fn on_needs_sync(&mut self, local_height: NodeHeight, remote_qc_height: NodeHeight) {
        self.diagnostics.emit(diag_event!(
            warn,
            "consensus.needs_sync",
            "Behind the committee at height {local_height} (peer certificate at {remote_qc_height})",
            local_height => local_height,
            remote_qc_height => remote_qc_height
        ));
    }

    fn on_state_transition(
        &mut self,
        from: ConsensusCurrentState,
        to: ConsensusCurrentState,
        event: &ConsensusStateEvent,
    ) {
        let (topic, level) = classify(from, to, event);
        self.diagnostics.emit(
            DiagnosticEvent::new(level, topic, format!("Consensus moved from {from} to {to} ({event})"))
                .with_field("from", from)
                .with_field("to", to)
                .with_field("event", event),
        );
    }

    fn on_no_vote(&mut self, block_id: &BlockId, reason: &NoVoteReason) {
        self.diagnostics.emit(diag_event!(
            warn,
            "consensus.no_vote",
            "Did not vote on block {block_id}: {reason}",
            block_id => block_id,
            reason => reason
        ));
    }

    /// Fires only on the first evidence per view and signer, so an equivocator cannot flood the log.
    fn on_vote_equivocation(&mut self, evidence: &VoteEquivocation) {
        self.diagnostics.emit(diag_event!(
            error,
            "consensus.vote_equivocation",
            "{evidence}",
            epoch => evidence.epoch,
            height => evidence.height,
            public_key => evidence.public_key,
            first_block_id => evidence.first.block_id,
            second_block_id => evidence.second.block_id
        ));
    }

    fn on_transaction_ready(&mut self, _tx_id: &TransactionId) {}

    fn on_transaction_batch_finalized(&mut self, _num_committed: usize, _num_aborted: usize) {}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StallAlarmKind {
    EpochHash,
    ProtocolVersion,
}

/// An error meaning consensus is stalled on this node until someone intervenes, reduced to what
/// distinguishes one stall from another.
///
/// `divergence` deliberately omits the block the error was raised on: a stalled node rejects a
/// different block every round while the divergence itself does not change, which is why the
/// worker's own guards key on `(epoch, local_epoch_hash, invalid_epoch_hash)` and
/// `(epoch, expected_version, block_version)` rather than on the error.
struct StallAlarm {
    kind: StallAlarmKind,
    divergence: String,
}

fn stall_alarm(err: &HotStuffError) -> Option<StallAlarm> {
    let HotStuffError::ProposalValidationError(err) = err else {
        return None;
    };
    match err {
        ProposalValidationError::InvalidEpochHash {
            epoch,
            local_epoch_hash,
            invalid_epoch_hash,
            ..
        } => Some(StallAlarm {
            kind: StallAlarmKind::EpochHash,
            divergence: format!("{epoch}:{local_epoch_hash}:{invalid_epoch_hash}"),
        }),
        ProposalValidationError::InvalidProtocolVersion {
            epoch,
            expected_version,
            block_version,
            ..
        } => Some(StallAlarm {
            kind: StallAlarmKind::ProtocolVersion,
            divergence: format!("{epoch}:{expected_version}:{block_version}"),
        }),
        _ => None,
    }
}

/// Mirrors how consensus itself treats each error, so that `level = error` means something an
/// operator should look at rather than a condition consensus already handles.
///
/// `handle_hotstuff_error` reports every error it sees, including the ones it goes on to resolve by
/// catching up, and it resolves those once per proposal for as long as the node is behind.
fn classify_error(err: &HotStuffError) -> (&'static str, DiagnosticLevel) {
    if stall_alarm(err).is_some() {
        return ("consensus.error", DiagnosticLevel::Error);
    }

    // A missing justify block puts the node on the same catch-up path as an explicit
    // `FallenBehind`, even though it is not part of `is_sync_required`.
    if err.is_sync_required() ||
        matches!(
            err,
            HotStuffError::ProposalValidationError(ProposalValidationError::JustifyBlockNotFound { .. })
        )
    {
        return ("consensus.needs_sync", DiagnosticLevel::Warn);
    }

    if matches!(err, HotStuffError::ProposalValidationError(_)) {
        return ("consensus.block_validation_failed", DiagnosticLevel::Warn);
    }

    ("consensus.error", DiagnosticLevel::Error)
}

fn classify(
    from: ConsensusCurrentState,
    to: ConsensusCurrentState,
    event: &ConsensusStateEvent,
) -> (&'static str, DiagnosticLevel) {
    use tari_ootle_common_types::diagnostics::DiagnosticLevel::{Error, Info};

    match (from, to, event) {
        // `Sleeping` is only ever entered from a failure, and is where an operator looks first when
        // a node has gone quiet.
        (_, ConsensusCurrentState::Sleeping, _) => ("consensus.crashed", Error),
        (_, ConsensusCurrentState::Syncing, _) => ("sync.started", Info),
        // The arms above have already taken the failure path out of `Syncing`, so a shutdown is the
        // only remaining way to leave it other than by finishing.
        (ConsensusCurrentState::Syncing, ConsensusCurrentState::Shutdown, _) => ("consensus.state_transition", Info),
        (ConsensusCurrentState::Syncing, _, _) => ("sync.completed", Info),
        _ => ("consensus.state_transition", Info),
    }
}

#[cfg(test)]
mod tests {
    use tari_consensus_types::LeafBlock;
    use tari_engine_types::ProtocolVersion;
    use tari_ootle_common_types::{Epoch, NumPreshards, ShardGroup};
    use tari_sidechain::FixedHash;
    use tokio::sync::mpsc;

    use super::*;

    fn hooks() -> (DiagnosticHooks, mpsc::Receiver<DiagnosticEvent>) {
        let (tx, rx) = mpsc::channel(16);
        (
            DiagnosticHooks::new(DiagnosticsHandle::new(tx, DiagnosticLevel::Info)),
            rx,
        )
    }

    fn drain(rx: &mut mpsc::Receiver<DiagnosticEvent>) -> Vec<DiagnosticEvent> {
        let mut events = Vec::new();
        while let Ok(event) = rx.try_recv() {
            events.push(event);
        }
        events
    }

    fn invalid_epoch_hash(block: u8, invalid_hash: u8) -> HotStuffError {
        HotStuffError::ProposalValidationError(ProposalValidationError::InvalidEpochHash {
            block_id: BlockId::new(FixedHash::from([block; 32])),
            epoch: Epoch(7),
            local_epoch_hash: FixedHash::from([1u8; 32]),
            invalid_epoch_hash: FixedHash::from([invalid_hash; 32]),
        })
    }

    fn invalid_protocol_version(block: u8) -> HotStuffError {
        HotStuffError::ProposalValidationError(ProposalValidationError::InvalidProtocolVersion {
            expected_version: ProtocolVersion::V0,
            block_version: ProtocolVersion::V0,
            epoch: Epoch(7),
            block_id: BlockId::new(FixedHash::from([block; 32])),
        })
    }

    #[test]
    fn one_divergence_records_once_however_many_blocks_it_rejects() {
        let (mut hooks, mut rx) = hooks();
        hooks.on_error(&invalid_epoch_hash(1, 9));
        hooks.on_error(&invalid_epoch_hash(2, 9));
        hooks.on_error(&invalid_epoch_hash(3, 9));

        let events = drain(&mut rx);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].topic, "consensus.error");
        assert_eq!(events[0].level, DiagnosticLevel::Error);
    }

    #[test]
    fn a_different_divergence_records_again() {
        let (mut hooks, mut rx) = hooks();
        hooks.on_error(&invalid_epoch_hash(1, 9));
        hooks.on_error(&invalid_epoch_hash(2, 10));

        assert_eq!(drain(&mut rx).len(), 2);
    }

    #[test]
    fn the_two_alarm_kinds_do_not_evict_each_other() {
        let (mut hooks, mut rx) = hooks();
        hooks.on_error(&invalid_epoch_hash(1, 9));
        hooks.on_error(&invalid_protocol_version(2));
        hooks.on_error(&invalid_epoch_hash(3, 9));
        hooks.on_error(&invalid_protocol_version(4));

        // Each kind keeps its own slot, so the interleaving records one row per divergence, not four.
        assert_eq!(drain(&mut rx).len(), 2);
    }

    #[test]
    fn errors_sync_resolves_are_not_recorded_as_errors() {
        let (mut hooks, mut rx) = hooks();
        hooks.on_error(&HotStuffError::ProposalValidationError(
            ProposalValidationError::JustifyBlockNotFound {
                proposed_by: "peer".to_string(),
                block_description: "block".to_string(),
                justify_block: LeafBlock {
                    block_id: BlockId::zero(),
                    height: NodeHeight(1),
                    epoch: Epoch(7),
                    shard_group: ShardGroup::all_shards(NumPreshards::P1),
                },
            },
        ));

        let events = drain(&mut rx);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].topic, "consensus.needs_sync");
        assert_eq!(events[0].level, DiagnosticLevel::Warn);
    }
}
