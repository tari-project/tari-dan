//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause
use prometheus::{IntCounter, IntGauge, IntGaugeVec, Opts, Registry};
use tari_consensus::{hotstuff::HotStuffError, messages::HotstuffMessage, traits::hooks::ConsensusHooks};
use tari_dan_common_types::NodeHeight;
use tari_dan_storage::consensus_models::{Decision, QuorumDecision, TransactionAtom, ValidBlock};
use tari_transaction::TransactionId;

use crate::metrics::CollectorRegister;

#[derive(Debug, Clone)]
pub struct PrometheusConsensusMetrics {
    local_blocks_received: IntCounter,
    blocks_accepted: IntCounter,
    blocks_rejected: IntCounter,
    blocks_validation_failed: IntCounter,

    commands_count: IntGaugeVec,

    messages_received: IntCounter,

    errors: IntCounter,

    pacemaker_height: IntGauge,
    pacemaker_leader_failures: IntCounter,
    needs_sync: IntCounter,

    transactions_ready_for_consensus: IntCounter,
    transactions_finalized_committed: IntCounter,
    transactions_finalized_aborted: IntCounter,
}

impl PrometheusConsensusMetrics {
    pub fn new(registry: &Registry) -> Self {
        Self {
            local_blocks_received: IntCounter::new("consensus_blocks_received", "Number of blocks added")
                .unwrap()
                .register_at(registry),
            blocks_accepted: IntCounter::new("consensus_blocks_accepted", "Number of blocks accepted")
                .unwrap()
                .register_at(registry),
            commands_count: IntGaugeVec::new(Opts::new("consensus_num_commands", "Number of commands added"), &[
                "block_id",
            ])
            .unwrap()
            .register_at(registry),
            messages_received: IntCounter::new("consensus_messages_received", "Number of messages received")
                .unwrap()
                .register_at(registry),
            errors: IntCounter::new("consensus_errors", "Number of errors")
                .unwrap()
                .register_at(registry),
            pacemaker_height: IntGauge::new("consensus_pacemaker_height", "Current pacemaker height")
                .unwrap()
                .register_at(registry),
            pacemaker_leader_failures: IntCounter::new("consensus_leader_failures", "Number of leader failures")
                .unwrap()
                .register_at(registry),
            blocks_validation_failed: IntCounter::new(
                "consensus_block_validation_failed",
                "Number of block validation failures",
            )
            .unwrap()
            .register_at(registry),
            blocks_rejected: IntCounter::new("consensus_blocks_rejected", "Number of blocks rejected")
                .unwrap()
                .register_at(registry),
            needs_sync: IntCounter::new("consensus_needs_sync", "Number of times consensus needs to sync")
                .unwrap()
                .register_at(registry),
            transactions_ready_for_consensus: IntCounter::new(
                "consensus_transaction_ready_for_consensus",
                "Number of transactions ready for consensus",
            )
            .unwrap()
            .register_at(registry),
            transactions_finalized_committed: IntCounter::new(
                "consensus_transaction_finalized_committed",
                "Number of committed transactions",
            )
            .unwrap()
            .register_at(registry),
            transactions_finalized_aborted: IntCounter::new(
                "consensus_transaction_finalized_aborted",
                "Number of aborted transactions",
            )
            .unwrap()
            .register_at(registry),
        }
    }
}

impl ConsensusHooks for PrometheusConsensusMetrics {
    fn on_local_block_decide(&mut self, block: &ValidBlock, decision: Option<QuorumDecision>) {
        self.local_blocks_received.inc();
        match decision {
            Some(QuorumDecision::Accept) => {
                self.commands_count
                    .with_label_values(&[&block.block().id().to_string()])
                    .set(block.block().commands().len() as i64);
                self.blocks_accepted.inc();
            },
            Some(QuorumDecision::Reject) | None => {
                self.blocks_rejected.inc();
            },
        }
    }

    fn on_block_validation_failed<E: ToString>(&mut self, _err: &E) {
        self.blocks_validation_failed.inc();
    }

    fn on_message_received(&mut self, _message: &HotstuffMessage) {
        self.messages_received.inc();
    }

    fn on_error(&mut self, _err: &HotStuffError) {
        self.errors.inc();
    }

    fn on_pacemaker_height_changed(&mut self, height: NodeHeight) {
        self.pacemaker_height.set(height.as_u64() as i64);
    }

    fn on_leader_timeout(&mut self, _new_height: NodeHeight) {
        self.pacemaker_leader_failures.inc()
    }

    fn on_needs_sync(&mut self, _local_height: NodeHeight, _remote_qc_height: NodeHeight) {
        self.needs_sync.inc();
    }

    fn on_transaction_ready(&mut self, _tx_id: &TransactionId) {
        self.transactions_ready_for_consensus.inc();
    }

    fn on_transaction_finalized(&mut self, transaction: &TransactionAtom) {
        match transaction.decision {
            Decision::Commit => {
                self.transactions_finalized_committed.inc();
            },
            Decision::Abort => {
                self.transactions_finalized_aborted.inc();
            },
        }
    }
}
