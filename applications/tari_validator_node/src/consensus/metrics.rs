//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashMap, str::FromStr};

use prometheus::{core::Collector, IntCounter, IntGauge, IntGaugeVec, Opts, Registry};
use tari_consensus::{hotstuff::HotStuffError, messages::HotstuffMessage, traits::hooks::ConsensusHooks};
use tari_dan_common_types::{NodeHeight, PeerAddress};
use tari_dan_storage::{
    consensus_models::{Decision, QuorumDecision, TransactionAtom, ValidBlock},
    StateStore,
};
use tari_state_store_sqlite::SqliteStateStore;
use tari_transaction::TransactionId;

use crate::metrics::{CollectorRegister, LabelledCollector};

#[derive(Debug, Clone)]
pub struct PrometheusConsensusMetrics<S = SqliteStateStore<PeerAddress>> {
    _state_store: S,
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

    _transactions_pool_size: IntGauge,
    transactions_ready_for_consensus: IntCounter,
    transactions_finalized_committed: IntCounter,
    transactions_finalized_aborted: IntCounter,
}

impl<S: StateStore> PrometheusConsensusMetrics<S> {
    pub fn new(state_store: S, registry: &Registry) -> Self {
        Self {
            _state_store: state_store,
            local_blocks_received: IntCounter::new("consensus_blocks_received", "Number of blocks added")
                .unwrap()
                .register_at(registry),
            blocks_accepted: IntCounter::new("consensus_blocks_accepted", "Number of blocks accepted")
                .unwrap()
                .register_at(registry),
            commands_count: IntGaugeVec::new(Opts::new("consensus_num_commands", "Number of commands added"), &[
                "block_height",
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
            _transactions_pool_size: IntGauge::new(
                "consensus_transactions_pool_size",
                "Number of transactions in pool",
            )
                .unwrap()
                .register_at(registry),
        }
    }

    fn clean_up_commands_count(&self, prune_height: u64) {
        let metrics = self.commands_count.collect();
        let mut labels_to_remove = HashMap::new();

        for metric_fam in &metrics {
            labels_to_remove.clear();
            for m in metric_fam.get_metric() {
                let labels = m.get_label();
                if let Some(l) = labels.iter().find(|l| l.get_name() == "block_height") {
                    if u64::from_str(l.get_value()).unwrap_or(0) < prune_height {
                        labels_to_remove.extend(labels.iter().map(|l| (l.get_name(), l.get_value())));
                    }
                }
            }

            if labels_to_remove.is_empty() {
                continue;
            }
            self.commands_count.remove(&labels_to_remove).unwrap();
        }
    }
}

impl<S: StateStore> ConsensusHooks for PrometheusConsensusMetrics<S> {
    fn on_local_block_decide(&mut self, block: &ValidBlock, decision: Option<QuorumDecision>) {
        self.local_blocks_received.inc();
        match decision {
            Some(QuorumDecision::Accept) => {
                if !block.block().commands().is_empty() {
                    // Cleanup command count
                    let prune_height = block.height().as_u64().saturating_sub(100);
                    self.clean_up_commands_count(prune_height);

                    self.commands_count
                        .with_two_labels(&block.height().as_u64(), block.id())
                        .set(block.block().commands().len() as i64);
                }
                self.blocks_accepted.inc();
            }
            Some(QuorumDecision::Reject) | None => {
                self.blocks_rejected.inc();
            }
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
            }
            Decision::Abort(_) => {
                self.transactions_finalized_aborted.inc();
            }
        }
    }
}
