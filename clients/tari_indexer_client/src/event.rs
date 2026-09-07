//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::sync::Arc;

use tari_engine_types::transaction_receipt::FinalizeOutcome;
use tari_ootle_common_types::Epoch;
use tari_ootle_transaction::TransactionId;

#[derive(Debug, Clone)]
pub enum IndexerEvent {
    NewEpoch(NewEpochEvent),
    TransactionFinalized(TransactionFinalizedEvent),
}

impl IndexerEvent {
    /// SSE event name of [`IndexerEvent::NewEpoch`].
    pub const NEW_EPOCH_EVENT_NAME: &'static str = "NewEpoch";
    /// SSE event name of [`IndexerEvent::TransactionFinalized`].
    pub const TRANSACTION_FINALIZED_EVENT_NAME: &'static str = "TransactionFinalized";

    pub const fn as_event_name(&self) -> &'static str {
        match self {
            Self::NewEpoch(_) => Self::NEW_EPOCH_EVENT_NAME,
            Self::TransactionFinalized(_) => Self::TRANSACTION_FINALIZED_EVENT_NAME,
        }
    }
}

impl From<NewEpochEvent> for IndexerEvent {
    fn from(event: NewEpochEvent) -> Self {
        Self::NewEpoch(event)
    }
}

impl From<TransactionFinalizedEvent> for IndexerEvent {
    fn from(event: TransactionFinalizedEvent) -> Self {
        Self::TransactionFinalized(event)
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NewEpochEvent {
    pub epoch: Epoch,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TransactionFinalizedEvent {
    pub transaction_id: TransactionId,
    pub outcome: FinalizeOutcome,
}

/// A template-emitted event with its originating transaction ID.
/// Streamed via the /transactions/events/stream SSE endpoint.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TransactionEvent {
    /// The database auto-increment ID for this event, used as the SSE event ID for catch-up/replay.
    /// Excluded from the JSON payload — it is transmitted via the SSE `id:` field instead.
    #[serde(skip)]
    pub id: i64,
    pub transaction_id: TransactionId,
    pub event: Arc<tari_engine_types::events::Event>,
}
