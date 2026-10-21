//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_ootle_storage::time::PrimitiveDateTime;

use crate::storage_sqlite::schema::key_values;

pub struct KeyValue<T> {
    pub _key: String,
    pub value: T,
    pub _created_at: PrimitiveDateTime,
    pub _updated_at: PrimitiveDateTime,
}

impl From<KeyValueEntry> for KeyValue<String> {
    fn from(entry: KeyValueEntry) -> Self {
        KeyValue {
            _key: entry.key,
            value: entry.value,
            _created_at: entry.created_at,
            _updated_at: entry.updated_at,
        }
    }
}

#[derive(Debug, Identifiable, Queryable)]
#[diesel(table_name = key_values)]
pub(crate) struct KeyValueEntry {
    pub id: i64,
    pub key: String,
    pub value: String,
    pub created_at: PrimitiveDateTime,
    pub updated_at: PrimitiveDateTime,
}

pub enum Key {
    /// The network that applies to this database. Used to determine if the network has changed.
    /// type: Network
    Network,
    /// A summary of the sync progress. Used to resume sync after restarts.
    /// type: SyncProgress
    SyncProgress,
    /// The total accumulated amount of TARI that has been burned as exhaust, sourced from checkpoint
    /// headers. This is the authoritative, complete-since-genesis burn total used for supply.
    /// type: Amount
    TariAccumulatedExhaustBurn,
    /// The total accumulated amount of TARI that has been claimed.
    /// type: Amount
    TariAccumulatedClaimed,
    /// The total accumulated fees paid by transaction payers, summed from transaction receipts as
    /// `total_fees_paid`. Paired with `TariAccumulatedReceiptExhaustBurn` (same receipt source) it yields the
    /// realized burn share `burn / paid`.
    /// type: Amount
    TariAccumulatedFees,
    /// The total accumulated exhaust burn summed from transaction receipts (`exhaust_burn`). Unlike
    /// `TariAccumulatedExhaustBurn` (header-sourced, authoritative for supply) this shares the receipt source
    /// with `TariAccumulatedFees`, so their ratio is the exact realized rate; comparing the two burn totals
    /// reveals receipts the indexer has not observed (pruned/lagging).
    /// type: Amount
    TariAccumulatedReceiptExhaustBurn,
    /// The number of transaction receipts this indexer has indexed, including those it has since pruned.
    /// type: u64
    TransactionReceiptCount,
}

impl Key {
    pub fn as_key_str(&self) -> &'static str {
        match self {
            Self::Network => "network",
            Self::SyncProgress => "sync_progress",
            Self::TariAccumulatedClaimed => "tari_accumulated_claimed",
            Self::TariAccumulatedExhaustBurn => "tari_accumulated_exhaust_burn",
            Self::TariAccumulatedFees => "tari_accumulated_fees",
            Self::TariAccumulatedReceiptExhaustBurn => "tari_accumulated_receipt_exhaust_burn",
            Self::TransactionReceiptCount => "transaction_receipt_count",
        }
    }
}

impl AsRef<str> for Key {
    fn as_ref(&self) -> &str {
        self.as_key_str()
    }
}
