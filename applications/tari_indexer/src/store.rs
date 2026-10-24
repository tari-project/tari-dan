//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::HashMap,
    ops::{Deref, DerefMut},
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use serde::{Serialize, de::DeserializeOwned};
use tari_engine_types::{
    Utxo,
    encoded::Encoded,
    events::Event,
    fees::FeeReceipt,
    published_template::PublishedTemplateMetadata,
    substate::{EncodedSubstate, SubstateId},
    transaction_receipt::{FinalizeOutcome, TransactionReceipt},
};
use tari_indexer_client::types::{TransactionEntry, TransactionSource, UtxoStateUpdateSet};
use tari_indexer_lib::substate_cache::{FetchWatermark, SubstateCacheEntry, SubstateCacheEntryRef};
use tari_ootle_common_types::{
    Epoch,
    NodeHeight,
    ShardGroup,
    StateVersion,
    SubstateVersion,
    optional::Optional,
    shard::Shard,
};
use tari_ootle_storage::{
    Ordering,
    StorageError,
    consensus_models::{EpochCheckpoint, SubstateData},
    time::PrimitiveDateTime,
};
use tari_ootle_transaction::{Transaction, TransactionId};
use tari_sidechain::FixedHash;
use tari_template_lib_types::{
    Amount,
    ResourceAddress,
    TemplateAddress,
    TransactionReceiptAddress,
    UtxoAddress,
    UtxoId,
    crypto::{RistrettoPublicKeyBytes, UtxoTag},
};
use tari_validator_node_rpc::client::SubstateProofData;

use crate::{
    network_state_sync::{EventFilter, SyncProgress},
    storage_sqlite::models::{
        Key,
        KeyValue,
        SubstateCacheInvalidation,
        SubstateRecord,
        TemplateCatalogueEntry,
        UtxoUpdateRecord,
        VerifiedStateRoot,
        WatchedSubstateEntry,
    },
};

#[async_trait]
pub trait IndexerStore: IndexerStoreReader {
    type WriteTransaction<'a>: IndexerStoreWriteTransaction
        + Deref<Target = Self::ReadTransaction<'a>>
        + DerefMut
        + Send
    where Self: 'a;

    async fn with_write_tx<F, R, E>(&self, f: F) -> Result<R, E>
    where
        F: for<'a> FnOnce(&mut Self::WriteTransaction<'a>) -> Result<R, E> + Send + 'static,
        R: Send + 'static,
        E: From<StorageError> + Send + 'static;
}

#[async_trait]
pub trait IndexerStoreReader: Send + Sync + 'static {
    type ReadTransaction<'a>: IndexerStoreReadTransaction + Send
    where Self: 'a;

    async fn with_read_tx<F, R, E>(&self, f: F) -> Result<R, E>
    where
        F: for<'a> FnOnce(&mut Self::ReadTransaction<'a>) -> Result<R, E> + Send + 'static,
        R: Send + 'static,
        E: From<StorageError> + Send + 'static;
}

pub trait IndexerStoreReadTransaction {
    fn get_substate(
        &mut self,
        address: &SubstateId,
        version: Option<SubstateVersion>,
    ) -> Result<Option<SubstateRecord>, StorageError>;

    fn get_substates(&mut self, ids: &[SubstateId]) -> Result<HashMap<SubstateId, EncodedSubstate>, StorageError>;

    /// Returns the events matching `query` with id below `before_id` (or from the newest when
    /// `None`), newest first, skipping the first `offset` matches.
    fn get_events(
        &mut self,
        query: &EventQuery,
        before_id: Option<i64>,
        offset: u32,
        limit: u32,
    ) -> Result<EventsPage, StorageError>;

    /// Returns the events matching `query` with id above `after_id`, oldest first. Used for SSE
    /// catch-up/replay.
    fn get_events_after_id(
        &mut self,
        query: &EventQuery,
        after_id: i64,
        limit: u32,
    ) -> Result<EventsPage, StorageError>;

    /// Lists stored transactions newest first, optionally restricted to a single source.
    fn list_recent_transactions(
        &mut self,
        last_transaction_id: Option<TransactionId>,
        limit: usize,
        source: Option<TransactionSource>,
    ) -> Result<Vec<TransactionEntry>, StorageError>;

    /// Fetch a single transaction (with its instructions) by ID. Returns `None` if this indexer has
    /// no record of it: it was neither submitted here nor observed on the gossip topic, or it has
    /// aged past the retention window.
    fn get_transaction(&mut self, transaction_id: TransactionId) -> Result<Option<TransactionEntry>, StorageError>;

    /// Fetch the locally recorded rejection state of a transaction. Distinguishing "no row" from
    /// "row without a rejection" matters to callers that write a rejection: an `UPDATE` against a
    /// row that does not exist succeeds while changing nothing, so a caller that cannot tell the
    /// two apart would repeat that write on every read.
    fn get_transaction_rejection_status(
        &mut self,
        transaction_id: TransactionId,
    ) -> Result<TransactionRejectionStatus, StorageError>;

    // -------------------------------- Transaction Receipts -------------------------------- //
    fn list_transaction_receipts(
        &mut self,
        last_id: Option<TransactionReceiptAddress>,
        limit: u64,
        ordering: Ordering,
    ) -> Result<Vec<(TransactionReceiptAddress, Encoded<TransactionReceipt>)>, StorageError>;

    fn get_transaction_receipt(
        &mut self,
        address: &TransactionReceiptAddress,
    ) -> Result<Encoded<TransactionReceipt>, StorageError>;

    /// The sum of every validator fee pool balance: the leader fees validators have earned and not yet claimed.
    fn sum_validator_fee_pool_balances(&mut self) -> Result<Amount, StorageError>;

    // -------------------------------- KeyValues -------------------------------- //
    fn key_value_get_value<K: AsRef<str>, T: DeserializeOwned>(&mut self, key: K) -> Result<T, StorageError>;
    fn key_value_get_raw<K: AsRef<str>>(&mut self, key: K) -> Result<KeyValue<String>, StorageError>;

    // -------------------------------- Epoch Checkpoints -------------------------------- //
    fn epoch_checkpoint_exists(&mut self, shard_group: ShardGroup, epoch: Epoch) -> Result<bool, StorageError>;
    /// Checkpoints from `from_epoch` on, ascending by epoch, of every shard group or of the shard groups that contain
    /// `shard`.
    fn epoch_checkpoint_get_all(
        &mut self,
        from_epoch: Epoch,
        shard: Option<Shard>,
        limit: u64,
    ) -> Result<Vec<EpochCheckpoint>, StorageError>;
    /// The checkpoint of the latest epoch, of any shard group or of the shard group that contains `shard`.
    fn epoch_checkpoint_get_latest(&mut self, shard: Option<Shard>) -> Result<EpochCheckpoint, StorageError>;

    // -------------------------------- UTXOs -------------------------------- //

    /// Reads up to `limit` of a shard's UTXO rows above `from_state_version`, in whole state versions,
    /// and returns the updates among them that pass `from_epoch` and `unspents_only`. A single state
    /// version wider than `limit` is read whole, overrunning it.
    fn utxos_get_updates(
        &mut self,
        resource_address: ResourceAddress,
        from_epoch: Epoch,
        shard: Shard,
        from_state_version: StateVersion,
        unspents_only: bool,
        limit: u32,
    ) -> Result<UtxoStateUpdateSet, StorageError>;

    fn utxos_list(
        &mut self,
        resource_address: &ResourceAddress,
        from_id: Option<UtxoId>,
        limit: u32,
    ) -> Result<Vec<(UtxoId, Encoded<Utxo>)>, StorageError>;

    fn utxos_get_unspent_by_public_nonce_and_tag(
        &mut self,
        resource_address: &ResourceAddress,
        public_nonce_and_tag: &[(UtxoTag, RistrettoPublicKeyBytes)],
    ) -> Result<Vec<(UtxoId, Encoded<Utxo>)>, StorageError>;

    /// The pairs of `public_nonce_and_tag` whose UTXO is spent or burnt.
    fn utxos_get_spent_public_nonce_and_tag(
        &mut self,
        resource_address: &ResourceAddress,
        public_nonce_and_tag: &[(UtxoTag, RistrettoPublicKeyBytes)],
    ) -> Result<Vec<(UtxoTag, RistrettoPublicKeyBytes)>, StorageError>;

    // -------------------------------- Template Catalogue -------------------------------- //

    fn list_template_catalogue(
        &mut self,
        name_filter: Option<&str>,
        after: Option<&TemplateAddress>,
        limit: u64,
    ) -> Result<Vec<TemplateCatalogueEntry>, StorageError>;

    fn get_template_catalogue_entry(
        &mut self,
        template_address: &TemplateAddress,
    ) -> Result<TemplateCatalogueEntry, StorageError>;

    // -------------------------------- Watched Substates -------------------------------- //

    fn list_watched_substates(
        &mut self,
        template_address: Option<&TemplateAddress>,
        limit: u64,
        offset: u64,
    ) -> Result<Vec<WatchedSubstateEntry>, StorageError>;

    // -------------------------------- Verified State Roots -------------------------------- //

    /// The height of the block `root` was recorded at, if it is a committee-validated state merkle
    /// root recorded for `(epoch, shard_group)`. This is the read-path trust decision: a hit lets a
    /// substate value proof be verified against `root` without re-validating the serving
    /// validator's commit proof.
    fn get_verified_state_root_height(
        &mut self,
        epoch: Epoch,
        shard_group: ShardGroup,
        root: &FixedHash,
    ) -> Result<Option<NodeHeight>, StorageError>;

    /// The height of the highest verified state root recorded at `epoch` for any shard group
    /// overlapping `shard_group`.
    fn get_latest_verified_height_overlapping(
        &mut self,
        epoch: Epoch,
        shard_group: ShardGroup,
    ) -> Result<Option<NodeHeight>, StorageError>;

    /// The most recently committed verified state root recorded for `(epoch, shard_group)`, if any.
    fn get_latest_verified_state_root(
        &mut self,
        epoch: Epoch,
        shard_group: ShardGroup,
    ) -> Result<Option<VerifiedStateRoot>, StorageError>;

    // -------------------------------- Substate Cache -------------------------------- //

    /// The cached head version of `substate_id`. Says nothing about whether it is fresh enough to
    /// serve - see [`crate::substate_cache::SqliteSubstateCache`] - nor what it implies for a
    /// lookup at a particular version, which is
    /// [`SubstateCacheEntry::answer_at`](tari_indexer_lib::substate_cache::SubstateCacheEntry::answer_at)'s
    /// to decide.
    fn substate_cache_get(&mut self, substate_id: &SubstateId) -> Result<Option<SubstateCacheEntry>, StorageError>;

    /// The proof held for `version` of `substate_id`. Whether that version is the cached head is
    /// [`substate_cache_get`](Self::substate_cache_get)'s to say.
    fn substate_cache_proof_get(
        &mut self,
        substate_id: &SubstateId,
        version: SubstateVersion,
    ) -> Result<Option<SubstateProofData>, StorageError>;
}

pub trait IndexerStoreWriteTransaction {
    fn commit(self) -> Result<(), StorageError>;
    fn rollback(self) -> Result<(), StorageError>;
    fn key_value_set<K: AsRef<str>, V: Serialize>(&mut self, key: K, value: V) -> Result<(), StorageError>;
    /// Applies each UTXO update, returning the addresses of unspent outputs skipped because an
    /// unspent row already holds their `(resource, public nonce, tag)`. The stored row is kept.
    fn batch_insert_utxo_updates<I: IntoIterator<Item = UtxoUpdateRecord>>(
        &mut self,
        epoch: Epoch,
        updates: I,
    ) -> Result<Vec<UtxoAddress>, StorageError>;
    fn upsert_substate(&mut self, substate: &SubstateData) -> Result<(), StorageError>;
    /// Inserts each receipt with its events, skipping any whose address already holds a receipt.
    /// A receipt address is written once on the network, so the stored receipt is kept: replacing
    /// it would let any later stream rewrite a transaction's recorded outcome.
    fn batch_insert_transaction_receipts<I: IntoIterator<Item = (TransactionReceiptAddress, TransactionReceipt)>>(
        &mut self,
        receipts: I,
        event_filters: &[EventFilter],
    ) -> Result<InsertedReceipts, StorageError>;
    /// Records a transaction submitted directly to this indexer. A row already stored from gossip is
    /// upgraded to [`TransactionSource::Local`]: the network gossips a submission straight back, and
    /// which of the two writes lands first is a race that must not decide the recorded source. Only
    /// the source is updated — `retention_epoch` may already carry a synced receipt's commit epoch.
    ///
    /// `retention_ceiling` caps the retention epoch recorded for a new row, so that a transaction
    /// declaring a distant `max_epoch` cannot claim a row the pruner never reaches.
    fn upsert_submitted_transaction(
        &mut self,
        transaction: &Transaction,
        retention_ceiling: Epoch,
    ) -> Result<(), StorageError>;
    /// Records transactions in the batch, ignoring those already stored, and returns the number of
    /// rows inserted. `retention_ceiling` is applied as in [`Self::upsert_submitted_transaction`].
    fn insert_batch_transactions<'a, I: IntoIterator<Item = &'a Transaction>>(
        &mut self,
        transactions: I,
        source: TransactionSource,
        retention_ceiling: Epoch,
    ) -> Result<usize, StorageError>;
    /// Mark a stored transaction as rejected by mempool validation, recording the reason.
    fn set_transaction_rejected(&mut self, transaction_id: TransactionId, reason: &str) -> Result<(), StorageError>;
    /// Clear a previous rejection, e.g. when the same transaction is later resubmitted successfully.
    fn clear_transaction_rejection(&mut self, transaction_id: TransactionId) -> Result<(), StorageError>;
    /// Deletes up to `limit` transactions retained past `cutoff`, oldest first, returning the number
    /// deleted. Transaction receipts are keyed independently of this table and are never removed here,
    /// so a pruned transaction still resolves to its receipt-backed outcome.
    fn prune_transactions_before_epoch(&mut self, cutoff: Epoch, limit: usize) -> Result<usize, StorageError>;
    /// Deletes up to `limit` transaction receipts that committed before `cutoff`, oldest first,
    /// returning the number deleted. The receipt-sourced totals are accumulated separately and keep
    /// counting a pruned receipt.
    fn prune_transaction_receipts_before_epoch(&mut self, cutoff: Epoch, limit: usize) -> Result<usize, StorageError>;
    /// Deletes up to `limit` events emitted by transactions that committed before `cutoff`, oldest
    /// first, returning the number deleted.
    fn prune_events_before_epoch(&mut self, cutoff: Epoch, limit: usize) -> Result<usize, StorageError>;
    fn insert_or_ignore_epoch_checkpoint(&mut self, epoch_checkpoint: &EpochCheckpoint) -> Result<(), StorageError>;
    fn upsert_template_catalogue(
        &mut self,
        template_address: &TemplateAddress,
        metadata: &PublishedTemplateMetadata,
    ) -> Result<(), StorageError>;
    /// Records a template's catalogue entry unless it already has one, returning whether it was
    /// inserted. A template is published once, so an entry already held is never rewritten.
    fn insert_or_ignore_template_catalogue(
        &mut self,
        template_address: &TemplateAddress,
        metadata: &PublishedTemplateMetadata,
    ) -> Result<bool, StorageError>;

    fn insert_watched_substate(
        &mut self,
        component_address: &SubstateId,
        template_address: &TemplateAddress,
    ) -> Result<(), StorageError>;

    fn delete_watched_substate(&mut self, component_address: &SubstateId) -> Result<(), StorageError>;

    /// Records a committee-validated state root, retaining only the most recent roots per
    /// `(epoch, shard_group)` (a bounded ring) so reads landing on a validator slightly behind the
    /// indexer's last probe still hit a trusted root. Idempotent on `(epoch, shard_group, root)`.
    fn upsert_verified_state_root(&mut self, root: &VerifiedStateRoot) -> Result<(), StorageError>;

    // -------------------------------- Substate Cache -------------------------------- //

    /// Records a substate's head version as fetched from a committee, unless a transition for it has
    /// been applied since `watermark` - in which case the fetch may have observed an older state than
    /// the cache already knows about, and nothing it returned can be trusted as current. Returns
    /// whether the entry was written.
    ///
    /// A version below the cached head is ignored, since it cannot be the head - but only while that
    /// head still has standing: `head_ttl` since it was recorded, and no proof behind the arriving
    /// result that the head itself lacks. A head is retired by no transition below it, so without
    /// those two escapes one recorded above the real version would stand until eviction.
    fn substate_cache_put(
        &mut self,
        substate_id: &SubstateId,
        entry: SubstateCacheEntryRef<'_>,
        watermark: FetchWatermark,
        head_ttl: Duration,
    ) -> Result<bool, StorageError>;

    /// Retires the cached heads these transitions supersede or destroy, and journals each substate at
    /// `state_version` so a fetch that started earlier cannot reinstate one.
    ///
    /// Must be applied in the same transaction that advances the sync watermark `state_version`
    /// belongs to: an entry is served on the argument that the cache holds every transition up to
    /// that watermark, which a reader observing one without the other would break.
    ///
    /// Returns how many cached entries were retired.
    fn substate_cache_invalidate<I: IntoIterator<Item = SubstateCacheInvalidation>>(
        &mut self,
        invalidations: I,
        state_version: StateVersion,
    ) -> Result<usize, StorageError>;

    /// Like [`substate_cache_invalidate`](Self::substate_cache_invalidate), but for transitions
    /// learnt of ahead of the stream - from a committee's finalized result - so the journalled
    /// `StateVersion` is not the transition's own. Each is journalled at its own version: the
    /// caller passes one just past the shard's watermark, so that every fetch captured before the
    /// stream delivers the transition is vetoed, and the stream's own journal row replaces it when
    /// it does.
    ///
    /// Returns how many cached entries were retired.
    fn substate_cache_retire_ahead<I: IntoIterator<Item = (SubstateCacheInvalidation, StateVersion)>>(
        &mut self,
        invalidations: I,
    ) -> Result<usize, StorageError>;

    /// Drops journal entries older than `journal_retention`, evicts the oldest cache entries down to
    /// `max_entries`, and drops the proofs of versions no cached head holds. Returns how many entries
    /// were evicted.
    fn substate_cache_prune(&mut self, journal_retention: Duration, max_entries: usize) -> Result<usize, StorageError>;
}

/// The locally recorded rejection state of a transaction.
#[derive(Debug, Clone)]
pub enum TransactionRejectionStatus {
    /// No row is stored for this transaction: this indexer has never seen it, or it has aged past
    /// the configured retention window and been pruned.
    NotStored,
    /// A row is stored, with no rejection recorded against it.
    NotRejected,
    /// A row is stored with a recorded rejection.
    Rejected {
        details: String,
        rejected_at: PrimitiveDateTime,
    },
}

/// Filters for the event queries. A `topic` containing `*` is matched segment-wise, as
/// [`EventFilter::topic_matches`] matches the live stream; any other topic must match exactly.
#[derive(Debug, Clone)]
pub struct EventQuery {
    pub topic: Option<String>,
    pub substate_id: Option<SubstateId>,
    pub template_address: Option<TemplateAddress>,
    pub resource_address: Option<ResourceAddress>,
    /// The most rows a wildcard topic query examines per call. A wildcard cannot use an index, so
    /// this bounds the call's cost however rare the matches are; `next_cursor` resumes the scan.
    pub wildcard_scan_limit: u32,
}

impl EventQuery {
    pub fn wildcard_topic(&self) -> Option<&str> {
        self.topic.as_deref().filter(|t| t.contains('*'))
    }
}

#[derive(Debug, Clone, Default)]
pub struct EventsPage {
    pub events: Vec<StoredEvent>,
    /// The id to resume from (`before_id` or `after_id`, matching the query's direction), or
    /// `None` once no further rows can match. A page can be shorter than the limit, or empty,
    /// and still carry a cursor when a wildcard scan stopped at its limit.
    pub next_cursor: Option<i64>,
}

/// An indexed event as stored: its CBOR encoding, with the columns it is queried by.
#[derive(Debug, Clone)]
pub struct StoredEvent {
    pub id: i64,
    pub transaction_id: TransactionId,
    pub topic: String,
    pub event: Encoded<Event>,
}

/// What [`IndexerStoreWriteTransaction::batch_insert_transaction_receipts`] wrote.
#[derive(Debug, Default)]
pub struct InsertedReceipts {
    pub receipts: Vec<InsertedReceipt>,
    /// Addresses that already held a receipt, which was kept as stored.
    pub skipped: Vec<TransactionReceiptAddress>,
    /// The events of the inserted receipts that matched the event filters.
    pub events: Vec<InsertedEvent>,
}

#[derive(Debug, Clone)]
pub struct InsertedReceipt {
    pub transaction_id: TransactionId,
    pub outcome: FinalizeOutcome,
    pub fee_receipt: FeeReceipt,
    /// Events the receipt carries, before the event filters.
    pub num_events: usize,
}

/// An event that was inserted into the database, with its assigned auto-increment ID.
#[derive(Debug, Clone)]
pub struct InsertedEvent {
    pub id: i64,
    pub transaction_id: TransactionId,
    pub event: Arc<Event>,
    /// `event` as stored.
    pub encoded: Encoded<Event>,
}

pub struct ReadOnlyStore<T: IndexerStoreReader> {
    inner: T,
}

/// Network-wide XTR economic totals accumulated during state sync.
pub struct XtrEconomics {
    /// Total XTR claimed (peg-in), accumulated from claimed-output tombstones.
    pub total_claimed: Amount,
    /// Total exhaust burned, sourced from checkpoint headers. Authoritative and complete since genesis.
    pub total_exhaust_burned: Amount,
    /// Total fees paid by transaction payers, summed from transaction receipts.
    pub fee_volume: Amount,
    /// Total exhaust burned, summed from the same transaction receipts as `fee_volume` (so their ratio is
    /// the exact realized burn share).
    pub receipt_exhaust_burned: Amount,
    /// Number of transaction receipts the indexer has indexed, including any it has since pruned.
    pub transaction_receipt_count: u64,
    /// Leader fees validators have earned and not yet claimed, summed over every validator fee pool.
    pub validator_claimable_fees: Amount,
}

impl<T: IndexerStoreReader + Clone> Clone for ReadOnlyStore<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T: IndexerStoreReader> ReadOnlyStore<T> {
    pub fn new(inner: T) -> Self {
        Self { inner }
    }

    pub async fn list_transaction_receipts(
        &self,
        last_id: Option<TransactionReceiptAddress>,
        limit: u64,
        ordering: Ordering,
    ) -> Result<Vec<(TransactionReceiptAddress, Encoded<TransactionReceipt>)>, StorageError> {
        self.inner
            .with_read_tx(move |tx| tx.list_transaction_receipts(last_id, limit, ordering))
            .await
    }

    pub async fn get_transaction_receipt(
        &self,
        address: &TransactionReceiptAddress,
    ) -> Result<Encoded<TransactionReceipt>, StorageError> {
        let address = *address;
        self.inner
            .with_read_tx(move |tx| tx.get_transaction_receipt(&address))
            .await
    }

    pub async fn get_tari_total_supply(&self) -> Result<Amount, StorageError> {
        self.inner
            .with_read_tx(|tx| {
                let claimed = tx
                    .key_value_get_value::<_, Amount>(Key::TariAccumulatedClaimed)
                    .optional()?
                    .unwrap_or_default();
                // Both `claimed` and the receipt-sourced burn advance on the state-sync frontier, so the
                // difference is internally consistent; the header-sourced total tracks a separate
                // (checkpoint) frontier and is kept only as a cross-check.
                let burnt = tx
                    .key_value_get_value::<_, Amount>(Key::TariAccumulatedReceiptExhaustBurn)
                    .optional()?
                    .unwrap_or_default();

                claimed
                    .checked_sub(burnt)
                    .ok_or_else(|| StorageError::DataInconsistency {
                        details: format!(
                            "XTR total supply underflow: claimed {} < total exhaust {}",
                            claimed, burnt
                        ),
                    })
            })
            .await
    }

    pub async fn get_tari_economics(&self) -> Result<XtrEconomics, StorageError> {
        self.inner
            .with_read_tx(|tx| {
                let total_claimed = tx
                    .key_value_get_value::<_, Amount>(Key::TariAccumulatedClaimed)
                    .optional()?
                    .unwrap_or_default();
                let total_exhaust_burned = tx
                    .key_value_get_value::<_, Amount>(Key::TariAccumulatedExhaustBurn)
                    .optional()?
                    .unwrap_or_default();
                let fee_volume = tx
                    .key_value_get_value::<_, Amount>(Key::TariAccumulatedFees)
                    .optional()?
                    .unwrap_or_default();
                let receipt_exhaust_burned = tx
                    .key_value_get_value::<_, Amount>(Key::TariAccumulatedReceiptExhaustBurn)
                    .optional()?
                    .unwrap_or_default();
                let transaction_receipt_count = tx
                    .key_value_get_value::<_, u64>(Key::TransactionReceiptCount)
                    .optional()?
                    .unwrap_or_default();
                let validator_claimable_fees = tx.sum_validator_fee_pool_balances()?;

                Ok(XtrEconomics {
                    total_claimed,
                    total_exhaust_burned,
                    fee_volume,
                    receipt_exhaust_burned,
                    transaction_receipt_count,
                    validator_claimable_fees,
                })
            })
            .await
    }

    pub async fn get_sync_progress(&self) -> Result<SyncProgress, StorageError> {
        self.inner
            .with_read_tx(|tx| tx.key_value_get_value(Key::SyncProgress))
            .await
    }

    pub async fn get_events(
        &self,
        query: EventQuery,
        before_id: Option<i64>,
        offset: u32,
        limit: u32,
    ) -> Result<EventsPage, StorageError> {
        self.inner
            .with_read_tx(move |tx| tx.get_events(&query, before_id, offset, limit))
            .await
    }

    pub async fn get_events_after_id(
        &self,
        query: EventQuery,
        after_id: i64,
        limit: u32,
    ) -> Result<EventsPage, StorageError> {
        self.inner
            .with_read_tx(move |tx| tx.get_events_after_id(&query, after_id, limit))
            .await
    }

    pub async fn list_template_catalogue(
        &self,
        name_filter: Option<&str>,
        after: Option<&TemplateAddress>,
        limit: u64,
    ) -> Result<Vec<crate::storage_sqlite::models::TemplateCatalogueEntry>, StorageError> {
        let name_filter = name_filter.map(str::to_owned);
        let after = after.copied();
        self.inner
            .with_read_tx(move |tx| tx.list_template_catalogue(name_filter.as_deref(), after.as_ref(), limit))
            .await
    }

    pub async fn get_template_catalogue_entry(
        &self,
        template_address: &TemplateAddress,
    ) -> Result<TemplateCatalogueEntry, StorageError> {
        let template_address = *template_address;
        self.inner
            .with_read_tx(move |tx| tx.get_template_catalogue_entry(&template_address))
            .await
    }

    pub async fn epoch_checkpoint_get_all(
        &self,
        from_epoch: Epoch,
        shard: Option<Shard>,
        limit: u64,
    ) -> Result<Vec<EpochCheckpoint>, StorageError> {
        self.inner
            .with_read_tx(move |tx| tx.epoch_checkpoint_get_all(from_epoch, shard, limit))
            .await
    }

    pub async fn epoch_checkpoint_get_latest(&self, shard: Option<Shard>) -> Result<EpochCheckpoint, StorageError> {
        self.inner
            .with_read_tx(move |tx| tx.epoch_checkpoint_get_latest(shard))
            .await
    }

    /// The latest checkpoint of the shard group holding each shard, each once and in shard order: the latest of the
    /// group holding the first shard, then of the group holding the shard after that group's last, until a shard that
    /// no stored checkpoint covers. A checkpoint read for a shard holds it, so each one read moves the walk forward.
    pub async fn epoch_checkpoint_get_latest_of_each_shard_group(&self) -> Result<Vec<EpochCheckpoint>, StorageError> {
        self.inner
            .with_read_tx(|tx| {
                let mut checkpoints = Vec::new();
                let mut next_shard = Some(Shard::first());
                while let Some(shard) = next_shard {
                    let Some(checkpoint) = tx.epoch_checkpoint_get_latest(Some(shard)).optional()? else {
                        break;
                    };
                    let shard_group =
                        checkpoint
                            .checked_shard_group()
                            .map_err(|e| StorageError::DataInconsistency {
                                details: format!(
                                    "stored checkpoint of {} has an invalid shard group: {e}",
                                    checkpoint.epoch()
                                ),
                            })?;
                    next_shard = shard_group.end().as_u32().checked_add(1).map(Shard::from);
                    checkpoints.push(checkpoint);
                }
                Ok(checkpoints)
            })
            .await
    }

    pub async fn list_watched_substates(
        &self,
        template_address: Option<&TemplateAddress>,
        limit: u64,
        offset: u64,
    ) -> Result<Vec<WatchedSubstateEntry>, StorageError> {
        let template_address = template_address.copied();
        self.inner
            .with_read_tx(move |tx| tx.list_watched_substates(template_address.as_ref(), limit, offset))
            .await
    }
}
