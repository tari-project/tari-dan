//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_common_types::NodeHeight;
use tari_dan_storage::consensus_models::{QuorumDecision, TransactionAtom, ValidBlock};
use tari_transaction::TransactionId;

use crate::{hotstuff::HotStuffError, messages::HotstuffMessage};

pub trait ConsensusHooks {
    fn on_local_block_decide(&mut self, block: &ValidBlock, decision: Option<QuorumDecision>);

    fn on_block_validation_failed<E: ToString>(&mut self, err: &E);
    fn on_message_received(&mut self, message: &HotstuffMessage);
    fn on_error(&mut self, err: &HotStuffError);
    fn on_pacemaker_height_changed(&mut self, height: NodeHeight);
    fn on_leader_timeout(&mut self, new_height: NodeHeight);

    fn on_needs_sync(&mut self, local_height: NodeHeight, remote_qc_height: NodeHeight);

    fn on_transaction_ready(&mut self, tx_id: &TransactionId);
    fn on_transaction_finalized(&mut self, transaction: &TransactionAtom);
}

#[derive(Debug, Clone)]
pub struct OptionalHooks<T> {
    inner: Option<T>,
}

impl<T> OptionalHooks<T> {
    pub fn enabled(inner: T) -> Self {
        Self { inner: Some(inner) }
    }

    pub fn disabled() -> Self {
        Self { inner: None }
    }
}

impl<T: ConsensusHooks> ConsensusHooks for OptionalHooks<T> {
    fn on_local_block_decide(&mut self, block: &ValidBlock, decision: Option<QuorumDecision>) {
        if let Some(inner) = self.inner.as_mut() {
            inner.on_local_block_decide(block, decision);
        }
    }

    fn on_block_validation_failed<E: ToString>(&mut self, err: &E) {
        if let Some(inner) = self.inner.as_mut() {
            inner.on_block_validation_failed(err);
        }
    }

    fn on_message_received(&mut self, message: &HotstuffMessage) {
        if let Some(inner) = self.inner.as_mut() {
            inner.on_message_received(message);
        }
    }

    fn on_error(&mut self, err: &HotStuffError) {
        if let Some(inner) = self.inner.as_mut() {
            inner.on_error(err);
        }
    }

    fn on_pacemaker_height_changed(&mut self, height: NodeHeight) {
        if let Some(inner) = self.inner.as_mut() {
            inner.on_pacemaker_height_changed(height);
        }
    }

    fn on_leader_timeout(&mut self, new_height: NodeHeight) {
        if let Some(inner) = self.inner.as_mut() {
            inner.on_leader_timeout(new_height);
        }
    }

    fn on_needs_sync(&mut self, local_height: NodeHeight, remote_qc_height: NodeHeight) {
        if let Some(inner) = self.inner.as_mut() {
            inner.on_needs_sync(local_height, remote_qc_height);
        }
    }

    fn on_transaction_ready(&mut self, tx_id: &TransactionId) {
        if let Some(inner) = self.inner.as_mut() {
            inner.on_transaction_ready(tx_id);
        }
    }

    fn on_transaction_finalized(&mut self, transaction: &TransactionAtom) {
        if let Some(inner) = self.inner.as_mut() {
            inner.on_transaction_finalized(transaction);
        }
    }
}

impl<T> From<T> for OptionalHooks<T> {
    fn from(inner: T) -> Self {
        Self { inner: Some(inner) }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NoopHooks;

impl ConsensusHooks for NoopHooks {
    fn on_local_block_decide(&mut self, _block: &ValidBlock, _decision: Option<QuorumDecision>) {}

    fn on_block_validation_failed<E: ToString>(&mut self, _: &E) {}

    fn on_message_received(&mut self, _message: &HotstuffMessage) {}

    fn on_error(&mut self, _err: &HotStuffError) {}

    fn on_pacemaker_height_changed(&mut self, _: NodeHeight) {}

    fn on_leader_timeout(&mut self, _new_height: NodeHeight) {}

    fn on_needs_sync(&mut self, _local_height: NodeHeight, _remote_qc_height: NodeHeight) {}

    fn on_transaction_ready(&mut self, _tx_id: &TransactionId) {}

    fn on_transaction_finalized(&mut self, _transaction: &TransactionAtom) {}
}
