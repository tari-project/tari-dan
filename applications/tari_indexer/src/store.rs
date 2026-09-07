//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::HashMap,
    ops::{Deref, DerefMut},
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use ootle_network::Network;
use serde::{Serialize, de::DeserializeOwned};
use tari_common_types::types::FixedHash;
use tari_engine_types::{
    Utxo,
    events::Event,
    published_template::PublishedTemplateMetadata,
    substate::{Substate, SubstateId},
    transaction_receipt::TransactionReceipt,
};
use tari_indexer_client::types::{
    ListSubstateItem,
    NonFungibleSubstate,
    TransactionEntry,
    TransactionSource,
    UtxoStateUpdateSet,
};
use tari_indexer_lib::substate_cache::{FetchWatermark, SubstateCacheEntry, SubstateCacheEntryRef};
use tari_ootle_common_types::{
    Epoch,
    ShardGroup,
    StateVersion,
    optional::Optional,
    shard::Shard,
    substate_type::SubstateType,
};
use tari_ootle_storage::{
    Ordering,
    StorageError,
    consensus_models::{EpochCheckpoint, SubstateData, SubstateUpdateProof},
    time::PrimitiveDateTime,
};
use tari_ootle_transaction::{Transaction, TransactionId};
use tari_template_lib_types::{
    Amount,
    ResourceAddress,
    TemplateAddress,
    TransactionReceiptAddress,
    UtxoId,
    crypto::{RistrettoPublicKeyBytes, UtxoTag},
};

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
    fn list_substates(
        &mut self,
        by_id: Option<&SubstateId>,
        filter_by_type: Option<SubstateType>,
        filter_by_template: Option<TemplateAddress>,
        limit: Option<u64>,
        offset: Option<u64>,
    ) -> Result<Vec<ListSubstateItem>, StorageError>;
    fn get_substate(
        &mut self,
        address: &SubstateId,
        version: Option<u32>,
    ) -> Result<Option<SubstateRecord>, StorageError>;

    fn get_substates(&mut self, ids: &[SubstateId]) -> Result<HashMap<SubstateId, Substate>, StorageError>;
    fn get_non_fungibles_by_resource_address(
        &mut self,
        resource_address: ResourceAddress,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<NonFungibleSubstate>, StorageError>;

    fn get_events(
        &mut self,
        substate_id_filter: Option<&SubstateId>,
        topic_filter: Option<&str>,
        resource_address_filter: Option<&ResourceAddress>,
        offset: u32,
        limit: u32,
    ) -> Result<Vec<(TransactionId, Event)>, StorageError>;

    /// Get events with id > after_id, ordered ascending by id.
    /// Used for SSE catch-up/replay.
    fn get_events_after_id(
        &mut self,
        after_id: i64,
        topic_filter: Option<&str>,
        substate_id_filter: Option<&SubstateId>,
        template_address_filter: Option<&TemplateAddress>,
        resource_address_filter: Option<&ResourceAddress>,
        limit: u32,
    ) -> Result<Vec<(i64, TransactionId, Event)>, StorageError>;

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
    ) -> Result<Vec<(TransactionReceiptAddress, TransactionReceipt)>, StorageError>;

    fn get_transaction_receipt(
        &mut self,
        address: &TransactionReceiptAddress,
    ) -> Result<TransactionReceipt, StorageError>;

    fn count_transaction_receipts(&mut self) -> Result<u64, StorageError>;

    // -------------------------------- KeyValues -------------------------------- //
    fn key_value_get_value<K: AsRef<str>, T: DeserializeOwned>(&mut self, key: K) -> Result<T, StorageError>;
    fn key_value_get_raw<K: AsRef<str>>(&mut self, key: K) -> Result<KeyValue<String>, StorageError>;

    // -------------------------------- Epoch Checkpoints -------------------------------- //
    fn epoch_checkpoint_exists(&mut self, shard_group: ShardGroup, epoch: Epoch) -> Result<bool, StorageError>;
    fn epoch_checkpoint_get_all(&mut self, from_epoch: Epoch, limit: u64)
    -> Result<Vec<EpochCheckpoint>, StorageError>;
    fn epoch_checkpoint_get_latest(&mut self) -> Result<EpochCheckpoint, StorageError>;

    // -------------------------------- UTXOs -------------------------------- //

    fn utxos_get_max_state_version(
        &mut self,
        resource_address: ResourceAddress,
        shard: Shard,
    ) -> Result<StateVersion, StorageError>;

    /// Get UTXO updates for a given resource address and shard, starting from a specific state version.
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
    ) -> Result<Vec<(UtxoId, Utxo)>, StorageError>;

    fn utxos_get_unspent_by_public_nonce_and_tag(
        &mut self,
        resource_address: &ResourceAddress,
        public_nonce_and_tag: &[(UtxoTag, RistrettoPublicKeyBytes)],
    ) -> Result<Vec<(UtxoId, Utxo)>, StorageError>;

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

    /// True if `root` is a committee-validated state merkle root recorded for `(epoch, shard_group)`.
    /// This is the read-path trust decision: a hit lets a substate value proof be verified against
    /// `root` without re-validating the serving validator's commit proof.
    fn is_state_root_trusted(
        &mut self,
        epoch: Epoch,
        shard_group: ShardGroup,
        root: &FixedHash,
    ) -> Result<bool, StorageError>;

    /// The most recently committed verified state root recorded for `(epoch, shard_group)`, if any.
    fn get_latest_verified_state_root(
        &mut self,
        epoch: Epoch,
        shard_group: ShardGroup,
    ) -> Result<Option<VerifiedStateRoot>, StorageError>;

    // -------------------------------- Substate Cache -------------------------------- //

    /// The cached head version of `substate_id`. Says nothing about whether it is fresh enough to
    /// serve, nor what it implies for lower versions - see [`crate::substate_cache::SqliteSubstateCache`].
    fn substate_cache_get(&mut self, substate_id: &SubstateId) -> Result<Option<SubstateCacheEntry>, StorageError>;
}

pub trait IndexerStoreWriteTransaction {
    fn commit(self) -> Result<(), StorageError>;
    fn rollback(self) -> Result<(), StorageError>;
    fn key_value_set<K: AsRef<str>, V: Serialize>(&mut self, key: K, value: V) -> Result<(), StorageError>;
    fn batch_insert_substate_transitions<I: IntoIterator<Item = (Epoch, SubstateUpdateProof)>>(
        &mut self,
        network: Network,
        shard: Shard,
        state_version: StateVersion,
        updates: I,
    ) -> Result<(), StorageError>;
    fn batch_insert_utxo_updates<I: IntoIterator<Item = UtxoUpdateRecord>>(
        &mut self,
        epoch: Epoch,
        updates: I,
    ) -> Result<(), StorageError>;
    fn upsert_substate(&mut self, substate: &SubstateData) -> Result<(), StorageError>;
    fn batch_insert_transaction_receipts<I: IntoIterator<Item = (TransactionReceiptAddress, TransactionReceipt)>>(
        &mut self,
        receipts: I,
        event_filters: &[EventFilter],
    ) -> Result<Vec<InsertedEvent>, StorageError>;
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
    fn insert_or_ignore_epoch_checkpoint(&mut self, epoch_checkpoint: &EpochCheckpoint) -> Result<(), StorageError>;
    fn upsert_template_catalogue(
        &mut self,
        template_address: &TemplateAddress,
        metadata: &PublishedTemplateMetadata,
    ) -> Result<(), StorageError>;

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
    fn substate_cache_invalidate<I: IntoIterator<Item = SubstateCacheInvalidation>>(
        &mut self,
        invalidations: I,
        state_version: StateVersion,
    ) -> Result<(), StorageError>;

    /// Like [`substate_cache_invalidate`](Self::substate_cache_invalidate), but for transitions
    /// learnt of ahead of the stream - from a committee's finalized result - so the journalled
    /// `StateVersion` is not the transition's own. Each is journalled at its own version: the
    /// caller passes one just past the shard's watermark, so that every fetch captured before the
    /// stream delivers the transition is vetoed, and the stream's own journal row replaces it when
    /// it does.
    fn substate_cache_retire_ahead<I: IntoIterator<Item = (SubstateCacheInvalidation, StateVersion)>>(
        &mut self,
        invalidations: I,
    ) -> Result<(), StorageError>;

    /// Drops journal entries older than `journal_retention` and evicts the oldest cache entries down
    /// to `max_entries`.
    fn substate_cache_prune(&mut self, journal_retention: Duration, max_entries: usize) -> Result<(), StorageError>;
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

/// An event that was inserted into the database, with its assigned auto-increment ID.
#[derive(Debug, Clone)]
pub struct InsertedEvent {
    pub id: i64,
    pub transaction_id: TransactionId,
    pub event: Arc<Event>,
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
    /// Number of transaction receipts the indexer has stored.
    pub transaction_receipt_count: u64,
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
    ) -> Result<Vec<(TransactionReceiptAddress, TransactionReceipt)>, StorageError> {
        self.inner
            .with_read_tx(move |tx| tx.list_transaction_receipts(last_id, limit, ordering))
            .await
    }

    pub async fn get_transaction_receipt(
        &self,
        address: &TransactionReceiptAddress,
    ) -> Result<TransactionReceipt, StorageError> {
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
                let transaction_receipt_count = tx.count_transaction_receipts()?;

                Ok(XtrEconomics {
                    total_claimed,
                    total_exhaust_burned,
                    fee_volume,
                    receipt_exhaust_burned,
                    transaction_receipt_count,
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
        substate_id_filter: Option<&SubstateId>,
        topic_filter: Option<&str>,
        resource_address_filter: Option<&ResourceAddress>,
        offset: u32,
        limit: u32,
    ) -> Result<Vec<(TransactionId, Event)>, StorageError> {
        let substate_id_filter = substate_id_filter.cloned();
        let topic_filter = topic_filter.map(str::to_owned);
        let resource_address_filter = resource_address_filter.copied();
        self.inner
            .with_read_tx(move |tx| {
                tx.get_events(
                    substate_id_filter.as_ref(),
                    topic_filter.as_deref(),
                    resource_address_filter.as_ref(),
                    offset,
                    limit,
                )
            })
            .await
    }

    pub async fn get_events_after_id(
        &self,
        after_id: i64,
        topic_filter: Option<&str>,
        substate_id_filter: Option<&SubstateId>,
        template_address_filter: Option<&TemplateAddress>,
        resource_address_filter: Option<&ResourceAddress>,
        limit: u32,
    ) -> Result<Vec<(i64, TransactionId, Event)>, StorageError> {
        let topic_filter = topic_filter.map(str::to_owned);
        let substate_id_filter = substate_id_filter.cloned();
        let template_address_filter = template_address_filter.copied();
        let resource_address_filter = resource_address_filter.copied();
        self.inner
            .with_read_tx(move |tx| {
                tx.get_events_after_id(
                    after_id,
                    topic_filter.as_deref(),
                    substate_id_filter.as_ref(),
                    template_address_filter.as_ref(),
                    resource_address_filter.as_ref(),
                    limit,
                )
            })
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
        limit: u64,
    ) -> Result<Vec<EpochCheckpoint>, StorageError> {
        self.inner
            .with_read_tx(move |tx| tx.epoch_checkpoint_get_all(from_epoch, limit))
            .await
    }

    pub async fn epoch_checkpoint_get_latest(&self) -> Result<EpochCheckpoint, StorageError> {
        self.inner.with_read_tx(|tx| tx.epoch_checkpoint_get_latest()).await
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
