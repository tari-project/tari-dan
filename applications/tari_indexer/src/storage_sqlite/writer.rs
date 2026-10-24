//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    ops::{Deref, DerefMut},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use diesel::{OptionalExtension, QueryDsl, RunQueryDsl, SqliteConnection};
use log::{debug, info, warn};
use serde::Serialize;
use tari_engine_types::{
    Utxo,
    encoded::Encoded,
    published_template::PublishedTemplateMetadata,
    substate::SubstateId,
    transaction_receipt::TransactionReceipt,
};
use tari_indexer_client::types::TransactionSource;
use tari_indexer_lib::substate_cache::{FetchWatermark, SubstateCacheEntryRef, caches_nonexistence};
use tari_ootle_common_types::{Epoch, StateVersion, displayable::Displayable, optional::Optional};
use tari_ootle_storage::{
    StorageError,
    consensus_models::{EpochCheckpoint, SubstateData},
    time::PrimitiveDateTime,
};
use tari_ootle_storage_sqlite::SqliteTransaction;
use tari_ootle_transaction::{Transaction, TransactionId};
use tari_template_lib_types::{TemplateAddress, TransactionReceiptAddress, UtxoAddress};
use tari_validator_node_rpc::client::{SubstateProofData, SubstateResult};

use crate::{
    diesel::ExpressionMethods,
    network_state_sync::EventFilter,
    storage_sqlite::{
        models::{
            Key,
            NewEvent,
            NewSubstate,
            NewTemplateCatalogueRow,
            NewTransaction,
            NewVerifiedStateRoot,
            NewWatchedSubstate,
            SubstateCacheInvalidation,
            SubstateRecord,
            UtxoRecordInsert,
            UtxoRecordUpdate,
            UtxoUpdateRecord,
            VerifiedStateRoot,
            encode_retention_epoch,
        },
        reader::SqliteStoreReadTransaction,
        serialization::{encode_committed, serialize_hex, serialize_json},
    },
    store::{
        IndexerStoreReadTransaction,
        IndexerStoreWriteTransaction,
        InsertedEvent,
        InsertedReceipt,
        InsertedReceipts,
    },
};

const LOG_TARGET: &str = "tari::indexer::storage_sqlite::writer";

fn unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

pub struct SqliteStoreWriteTransaction<'a> {
    /// None indicates if the transaction has been explicitly committed/rolled back
    transaction: Option<SqliteStoreReadTransaction<'a>>,
}

impl<'a> SqliteStoreWriteTransaction<'a> {
    pub fn new(transaction: SqliteTransaction<&'a mut SqliteConnection>) -> Self {
        Self {
            transaction: Some(SqliteStoreReadTransaction::new(transaction)),
        }
    }

    fn connection(&mut self) -> &mut SqliteConnection {
        self.transaction.as_mut().unwrap().connection()
    }
}
impl IndexerStoreWriteTransaction for SqliteStoreWriteTransaction<'_> {
    fn commit(mut self) -> Result<(), StorageError> {
        self.transaction.take().unwrap().transaction.commit()?;
        Ok(())
    }

    fn rollback(mut self) -> Result<(), StorageError> {
        self.transaction.take().unwrap().transaction.rollback()?;
        Ok(())
    }

    fn key_value_set<K: AsRef<str>, V: Serialize>(&mut self, key: K, value: V) -> Result<(), StorageError> {
        const OPERATION: &str = "key_value_set";
        use crate::storage_sqlite::schema::key_values;
        let json = serialize_json(&value)?;
        debug!(target: LOG_TARGET, "key_value_set called {} {}", key.as_ref(), json);

        diesel::insert_into(key_values::table)
            .values((key_values::key.eq(key.as_ref()), key_values::value.eq(&json)))
            .on_conflict(key_values::key)
            .do_update()
            .set(key_values::value.eq(&json))
            .execute(self.connection())
            .map_err(|e| StorageError::general(OPERATION, e))?;

        Ok(())
    }

    fn batch_insert_utxo_updates<I: IntoIterator<Item = UtxoUpdateRecord>>(
        &mut self,
        epoch: Epoch,
        updates: I,
    ) -> Result<Vec<UtxoAddress>, StorageError> {
        const OPERATION: &str = "batch_insert_utxo_updates";
        use crate::storage_sqlite::schema::utxos;

        let mut skipped = Vec::new();
        for update in updates {
            match update {
                UtxoUpdateRecord::Unspent(unspent) => {
                    let resource_address = unspent.address.resource_address().to_string();
                    let commitment = unspent.address.id().to_commitment_hex_string();

                    let insert = UtxoRecordInsert {
                        commitment,
                        public_nonce: serialize_hex(unspent.utxo_output.output.public_nonce),
                        version: unspent.version.as_u64() as i64,
                        utxo: Some(encode_committed(&Utxo {
                            output: Some(unspent.utxo_output.clone()),
                            is_frozen: unspent.is_frozen,
                        })?),
                        shard: unspent.shard.as_u32() as i32,
                        resource_address,
                        state_version: unspent.state_version.as_u64() as i64,
                        utxo_tag: unspent.utxo_output.tag.value() as i32,
                        epoch: epoch.as_u64() as i64,
                        is_spent: false,
                        is_burnt: false,
                    };
                    // batch insert results in "cannot start a transaction within a transaction" error
                    let num_inserted = diesel::insert_into(utxos::table)
                        .values(insert)
                        .on_conflict_do_nothing()
                        .execute(self.connection())
                        .map_err(|e| StorageError::general(OPERATION, format!("insert error: {e}")))?;
                    if num_inserted == 0 {
                        skipped.push(unspent.address);
                    }
                },
                UtxoUpdateRecord::Spent(spent) => {
                    let resource_address = spent.address.resource_address().to_string();
                    let commitment = spent.address.id().to_commitment_hex_string();
                    let update = UtxoRecordUpdate {
                        epoch: Some(epoch.as_u64() as i64),
                        version: Some(spent.version.as_u64() as i64),
                        // Prune the UTXO data for spent outputs
                        utxo: Some(None),
                        // Update to deleted state version
                        state_version: Some(spent.state_version.as_u64() as i64),
                        is_spent: Some(true),
                        ..Default::default()
                    };
                    diesel::update(utxos::table)
                        .filter(utxos::resource_address.eq(resource_address))
                        .filter(utxos::commitment.eq(commitment))
                        .set(update)
                        .execute(self.connection())
                        .map_err(|e| StorageError::general(OPERATION, format!("update error: {e}")))?;
                },
            }
        }

        Ok(skipped)
    }

    fn upsert_substate(&mut self, substate: &SubstateData) -> Result<(), StorageError> {
        use crate::storage_sqlite::schema::substates;

        let template_address = substate
            .value
            .value()
            .and_then(|s| s.component())
            .map(|c| c.template_address().to_string());
        let new_substate = NewSubstate {
            address: substate.substate_id.to_string(),
            version: substate.version.as_u64() as i64,
            data: substate
                .value
                .value()
                .map(encode_committed)
                .transpose()?
                .unwrap_or_default(),
            template_address,
            // Never set
            module_name: None,
        };

        let address = &new_substate.address;
        let current_substate = substates::table
            .filter(substates::address.eq(address))
            .first::<SubstateRecord>(self.connection())
            .optional()
            .map_err(|e| StorageError::QueryError {
                reason: format!("find_by_address: {}", e),
            })?;

        match current_substate {
            Some(_) => {
                diesel::update(substates::table)
                    .set(&new_substate)
                    .filter(substates::address.eq(address))
                    .execute(self.connection())
                    .map_err(|e| StorageError::QueryError {
                        reason: format!("Update leaf node: {}", e),
                    })?;
                debug!(
                    target: LOG_TARGET,
                    "Updated substate {} version to {}", address, new_substate.version
                );
            },
            None => {
                diesel::insert_into(substates::table)
                    .values(&new_substate)
                    .execute(self.connection())
                    .map_err(|e| StorageError::QueryError {
                        reason: format!("Update substate error: {}", e),
                    })?;
                info!(
                    target: LOG_TARGET,
                    "Added new substate {} with version {}", address, new_substate.version
                );
            },
        };

        Ok(())
    }

    fn batch_insert_transaction_receipts<I: IntoIterator<Item = (TransactionReceiptAddress, TransactionReceipt)>>(
        &mut self,
        receipts: I,
        event_filters: &[EventFilter],
    ) -> Result<InsertedReceipts, StorageError> {
        const OPERATION: &str = "batch_insert_transaction_receipts";
        use crate::storage_sqlite::schema::{events, transaction_receipts, transactions};

        let mut inserted = InsertedReceipts::default();

        for (receipt_addr, receipt) in receipts {
            let receipt_addr_hex = serialize_hex(receipt_addr.as_object_key());
            let transaction_id = TransactionId::from_receipt_address(receipt_addr);

            let num_inserted = diesel::insert_into(transaction_receipts::table)
                .values((
                    transaction_receipts::address.eq(&receipt_addr_hex),
                    transaction_receipts::data.eq(encode_committed(&receipt)?),
                    transaction_receipts::outcome.eq(receipt.outcome.to_string()),
                    transaction_receipts::total_fees_paid.eq(receipt.fee_receipt.total_fees_paid() as i64),
                    transaction_receipts::epoch.eq(encode_retention_epoch(receipt.epoch)),
                ))
                .on_conflict(transaction_receipts::address)
                .do_nothing()
                .execute(self.connection())
                .map_err(|e| StorageError::general(OPERATION, e))?;
            if num_inserted == 0 {
                inserted.skipped.push(receipt_addr);
                continue;
            }

            // The receipt carries the epoch the transaction committed in, which supersedes the
            // max_epoch recorded at submission as the retention key. Most synced receipts belong to
            // transactions submitted elsewhere and match no local row.
            diesel::update(transactions::table.filter(transactions::transaction_id.eq(&receipt_addr_hex)))
                .set(transactions::retention_epoch.eq(encode_retention_epoch(receipt.epoch)))
                .execute(self.connection())
                .map_err(|e| StorageError::general(OPERATION, e))?;

            // Insert events and collect assigned IDs
            let filtered_events: Vec<_> = receipt
                .events
                .iter()
                .filter(|event| event_filters.is_empty() || event_filters.iter().any(|filter| filter.matches(event)))
                .cloned()
                .collect();

            for event in filtered_events {
                let encoded = encode_committed(&event)?;
                let new_event = NewEvent {
                    template_address: event.template_address().to_string(),
                    tx_hash: &receipt_addr_hex,
                    topic: event.topic(),
                    event: encoded.clone(),
                    substate_id: event.substate_id().map(|s| s.to_string()),
                    resource_address: EventFilter::event_resource_address(&event).map(|r| r.to_string()),
                    epoch: encode_retention_epoch(receipt.epoch),
                };

                let id: i64 = diesel::insert_into(events::table)
                    .values(new_event)
                    .returning(events::id)
                    .get_result(self.connection())
                    .map_err(|e| StorageError::QueryError {
                        reason: format!("{OPERATION}: {}", e),
                    })?;

                inserted.events.push(InsertedEvent {
                    id,
                    transaction_id,
                    event: Arc::new(event),
                    encoded: Encoded::from_bytes(encoded),
                });
            }

            inserted.receipts.push(InsertedReceipt {
                transaction_id,
                outcome: receipt.outcome,
                fee_receipt: receipt.fee_receipt,
                num_events: receipt.events.len(),
            });
        }

        if !inserted.receipts.is_empty() {
            let count =
                Optional::optional(self.key_value_get_value::<_, u64>(Key::TransactionReceiptCount))?.unwrap_or(0);
            self.key_value_set(Key::TransactionReceiptCount, count + inserted.receipts.len() as u64)?;
        }

        Ok(inserted)
    }

    fn upsert_submitted_transaction(
        &mut self,
        transaction: &Transaction,
        retention_ceiling: Epoch,
    ) -> Result<(), StorageError> {
        use crate::storage_sqlite::schema::transactions;

        diesel::insert_into(transactions::table)
            .values(NewTransaction::new(transaction, TransactionSource::Local, retention_ceiling)?)
            .on_conflict(transactions::transaction_id)
            // Only the source: `retention_epoch` may already have been advanced to a synced
            // receipt's commit epoch, and the body is identical for a given transaction id.
            .do_update()
            .set(transactions::source.eq(TransactionSource::Local.as_str()))
            .execute(self.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("upsert_submitted_transaction: {e}"),
            })?;

        Ok(())
    }

    fn insert_batch_transactions<'i, I: IntoIterator<Item = &'i Transaction>>(
        &mut self,
        transactions: I,
        source: TransactionSource,
        retention_ceiling: Epoch,
    ) -> Result<usize, StorageError> {
        use crate::storage_sqlite::schema::transactions as transactions_table;

        let mut num_inserted = 0;
        // SQLite cannot express `ON CONFLICT` over a multi-row `VALUES` clause, so the rows are
        // inserted one statement at a time. The batching that matters is the enclosing write
        // transaction: it takes SQLite's database-wide write lock once for the whole flush.
        for transaction in transactions {
            let row = NewTransaction::new(transaction, source, retention_ceiling)?;
            num_inserted += diesel::insert_into(transactions_table::table)
                .values(row)
                .on_conflict_do_nothing()
                .execute(self.connection())
                .map_err(|e| StorageError::QueryError {
                    reason: format!("insert_batch_transactions: {e}"),
                })?;
        }

        Ok(num_inserted)
    }

    fn set_transaction_rejected(&mut self, transaction_id: TransactionId, reason: &str) -> Result<(), StorageError> {
        use crate::storage_sqlite::schema::transactions;

        diesel::update(transactions::table)
            .filter(transactions::transaction_id.eq(serialize_hex(transaction_id)))
            .set((
                transactions::rejected_reason.eq(reason),
                transactions::rejected_at.eq(diesel::dsl::now),
            ))
            .execute(self.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("set_transaction_rejected: {e}"),
            })?;

        Ok(())
    }

    fn clear_transaction_rejection(&mut self, transaction_id: TransactionId) -> Result<(), StorageError> {
        use crate::storage_sqlite::schema::transactions;

        diesel::update(transactions::table)
            .filter(transactions::transaction_id.eq(serialize_hex(transaction_id)))
            .filter(transactions::rejected_reason.is_not_null())
            .set((
                transactions::rejected_reason.eq(None::<String>),
                transactions::rejected_at.eq(None::<PrimitiveDateTime>),
            ))
            .execute(self.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("clear_transaction_rejection: {e}"),
            })?;

        Ok(())
    }

    fn prune_transactions_before_epoch(&mut self, cutoff: Epoch, limit: usize) -> Result<usize, StorageError> {
        const OPERATION: &str = "prune_transactions_before_epoch";
        use crate::storage_sqlite::schema::transactions;

        // Select then delete by id rather than issuing one open-ended range delete: SQLite holds a
        // single database-wide write lock for the duration of a statement, so an unbounded delete
        // over a large backlog would stall every other writer until it completes.
        //
        // Ordering by `retention_epoch` — the column the filter is on — is what lets SQLite serve
        // the select from `transactions_retention_epoch_idx` as a covering index. Ordering by any
        // other column makes it a full table scan on every call, including the common call that
        // finds nothing to prune.
        let ids = transactions::table
            .select(transactions::id)
            .filter(transactions::retention_epoch.lt(encode_retention_epoch(cutoff)))
            .order_by(transactions::retention_epoch.asc())
            .limit(limit as i64)
            .load::<i64>(self.connection())
            .map_err(|e| StorageError::general(OPERATION, e))?;

        if ids.is_empty() {
            return Ok(0);
        }

        let num_deleted = diesel::delete(transactions::table.filter(transactions::id.eq_any(ids)))
            .execute(self.connection())
            .map_err(|e| StorageError::general(OPERATION, e))?;

        Ok(num_deleted)
    }

    fn prune_transaction_receipts_before_epoch(&mut self, cutoff: Epoch, limit: usize) -> Result<usize, StorageError> {
        const OPERATION: &str = "prune_transaction_receipts_before_epoch";
        use crate::storage_sqlite::schema::transaction_receipts;

        // Bounded select-then-delete, ordered by the filtered column so the select is served from
        // `transaction_receipts_epoch_idx`. See `prune_transactions_before_epoch`.
        let ids = transaction_receipts::table
            .select(transaction_receipts::id)
            .filter(transaction_receipts::epoch.lt(encode_retention_epoch(cutoff)))
            .order_by(transaction_receipts::epoch.asc())
            .limit(limit as i64)
            .load::<i64>(self.connection())
            .map_err(|e| StorageError::general(OPERATION, e))?;

        if ids.is_empty() {
            return Ok(0);
        }

        diesel::delete(transaction_receipts::table.filter(transaction_receipts::id.eq_any(ids)))
            .execute(self.connection())
            .map_err(|e| StorageError::general(OPERATION, e))
    }

    fn prune_events_before_epoch(&mut self, cutoff: Epoch, limit: usize) -> Result<usize, StorageError> {
        const OPERATION: &str = "prune_events_before_epoch";
        use crate::storage_sqlite::schema::events;

        // Bounded select-then-delete, ordered by the filtered column so the select is served from
        // `events_epoch_idx`. See `prune_transactions_before_epoch`.
        let ids = events::table
            .select(events::id)
            .filter(events::epoch.lt(encode_retention_epoch(cutoff)))
            .order_by(events::epoch.asc())
            .limit(limit as i64)
            .load::<i64>(self.connection())
            .map_err(|e| StorageError::general(OPERATION, e))?;

        if ids.is_empty() {
            return Ok(0);
        }

        diesel::delete(events::table.filter(events::id.eq_any(ids)))
            .execute(self.connection())
            .map_err(|e| StorageError::general(OPERATION, e))
    }

    fn insert_or_ignore_epoch_checkpoint(&mut self, epoch_checkpoint: &EpochCheckpoint) -> Result<(), StorageError> {
        const OPERATION: &str = "insert_or_ignore_epoch_checkpoint";
        use crate::storage_sqlite::schema::epoch_checkpoints;

        let shard_group = epoch_checkpoint
            .checked_shard_group()
            .expect("shard group should be valid");
        diesel::insert_into(epoch_checkpoints::table)
            .values((
                epoch_checkpoints::epoch.eq(epoch_checkpoint.epoch().as_u64() as i64),
                epoch_checkpoints::shard_group.eq(shard_group.to_parsable_string()),
                epoch_checkpoints::shard_start.eq(shard_group.start().as_u32() as i32),
                epoch_checkpoints::shard_end.eq(shard_group.end().as_u32() as i32),
                epoch_checkpoints::json_data.eq(serialize_json(epoch_checkpoint)?),
            ))
            .on_conflict((epoch_checkpoints::epoch, epoch_checkpoints::shard_group))
            .do_nothing()
            .execute(self.connection())
            .map_err(|e| StorageError::general(OPERATION, e))?;

        Ok(())
    }

    fn upsert_template_catalogue(
        &mut self,
        template_address: &TemplateAddress,
        metadata: &PublishedTemplateMetadata,
    ) -> Result<(), StorageError> {
        const OPERATION: &str = "upsert_template_catalogue";
        use crate::storage_sqlite::schema::template_catalogue;

        let row = NewTemplateCatalogueRow::from((*template_address, metadata));

        diesel::insert_into(template_catalogue::table)
            .values(&row)
            .on_conflict(template_catalogue::template_address)
            .do_update()
            .set(&row)
            .execute(self.connection())
            .map_err(|e| StorageError::general(OPERATION, e))?;

        Ok(())
    }

    fn insert_or_ignore_template_catalogue(
        &mut self,
        template_address: &TemplateAddress,
        metadata: &PublishedTemplateMetadata,
    ) -> Result<bool, StorageError> {
        const OPERATION: &str = "insert_or_ignore_template_catalogue";
        use crate::storage_sqlite::schema::template_catalogue;

        let num_inserted = diesel::insert_into(template_catalogue::table)
            .values(&NewTemplateCatalogueRow::from((*template_address, metadata)))
            .on_conflict(template_catalogue::template_address)
            .do_nothing()
            .execute(self.connection())
            .map_err(|e| StorageError::general(OPERATION, e))?;

        Ok(num_inserted > 0)
    }

    fn insert_watched_substate(
        &mut self,
        component_address: &SubstateId,
        template_address: &TemplateAddress,
    ) -> Result<(), StorageError> {
        const OPERATION: &str = "insert_watched_substate";
        use crate::storage_sqlite::schema::watched_substates;

        let component_addr_str = component_address.to_string();
        let template_addr_str = template_address.to_string();

        diesel::insert_into(watched_substates::table)
            .values(NewWatchedSubstate {
                component_address: &component_addr_str,
                template_address: &template_addr_str,
            })
            .on_conflict(watched_substates::component_address)
            .do_update()
            .set(watched_substates::template_address.eq(&template_addr_str))
            .execute(self.connection())
            .map_err(|e| StorageError::general(OPERATION, e))?;

        Ok(())
    }

    fn delete_watched_substate(&mut self, component_address: &SubstateId) -> Result<(), StorageError> {
        const OPERATION: &str = "delete_watched_substate";
        use crate::storage_sqlite::schema::watched_substates;

        diesel::delete(
            watched_substates::table.filter(watched_substates::component_address.eq(component_address.to_string())),
        )
        .execute(self.connection())
        .map_err(|e| StorageError::general(OPERATION, e))?;

        Ok(())
    }

    fn substate_cache_put(
        &mut self,
        substate_id: &SubstateId,
        entry: SubstateCacheEntryRef<'_>,
        watermark: FetchWatermark,
        head_ttl: Duration,
    ) -> Result<bool, StorageError> {
        const OPERATION: &str = "substate_cache_put";
        use crate::storage_sqlite::schema::{substate_cache, substate_cache_invalidations};

        let id = substate_id.to_string();
        let version = entry.version.map(|v| v.as_u64() as i64);

        // Nothing journals a first creation for a substate outside `caches_nonexistence`, so a
        // record here that one does not exist could never be retracted and would stand until it
        // ages out. The invariant is enforced where it can be broken rather than only at the callers.
        if version.is_none() && !caches_nonexistence(substate_id) {
            warn!(
                target: LOG_TARGET,
                "Refusing to cache the nonexistence of {substate_id}: no transition would retract it"
            );
            return Ok(false);
        }

        // Read inside this transaction so that the journal and the insert cannot straddle a
        // concurrent invalidation commit.
        let journalled: Option<(i64, i64, bool)> = substate_cache_invalidations::table
            .select((
                substate_cache_invalidations::state_version,
                substate_cache_invalidations::substate_version,
                substate_cache_invalidations::spent,
            ))
            .filter(substate_cache_invalidations::substate_id.eq(&id))
            .first(self.connection())
            .optional()
            .map_err(|e| StorageError::general(OPERATION, e))?;

        if let Some((invalidated_at_version, observed_version, observed_version_spent)) = journalled {
            if invalidated_at_version as u64 > watermark.as_u64() {
                debug!(
                    target: LOG_TARGET,
                    "Discarding cache write for {substate_id} v{}: its shard advanced past the fetch",
                    entry.version.display()
                );
                return Ok(false);
            }

            // A fetch that started after the transition is not overtaken by it, but the member it
            // asked can be behind: it answers with a version the stream has already shown this
            // substate past. The transition left no cached row to rank that against, so the journal
            // is what refuses it.
            //
            // A record that the substate does not exist is left to the ranking below. It is settled
            // by f + 1 members rather than one, so a single member being behind cannot produce it,
            // and refusing it here would suppress the legitimate one that follows a destroy.
            if version.is_some_and(|v| v < observed_version) {
                debug!(
                    target: LOG_TARGET,
                    "Discarding cache write for {substate_id} v{}: the stream has already seen v{observed_version}",
                    entry.version.display()
                );
                return Ok(false);
            }

            // Where the journalled version is one the stream saw destroyed, it is the head only as a
            // `Down`, which is what a lookup for it answers. A member behind enough to still hold it
            // live offers an `Up` at the same version, and only the provenance tells the two apart.
            let claims_live_at_floor =
                version == Some(observed_version) && matches!(entry.substate_result, SubstateResult::Up { .. });
            if observed_version_spent && claims_live_at_floor {
                debug!(
                    target: LOG_TARGET,
                    "Discarding cache write for {substate_id} v{observed_version}: the stream saw that version spent"
                );
                return Ok(false);
            }
        }

        // A committee member that is behind can answer with a version below the head already held.
        let cached: Option<(Option<i64>, bool, i64)> = substate_cache::table
            .select((
                substate_cache::version,
                substate_cache::verified,
                substate_cache::cached_at,
            ))
            .filter(substate_cache::substate_id.eq(&id))
            .first(self.connection())
            .optional()
            .map_err(|e| StorageError::general(OPERATION, e))?;

        if let Some((cached_version, cached_verified, cached_at)) = cached {
            // A proof attests that a version existed, never that it is current, so a verified head is a
            // lower bound on the real one and nothing may walk it back: a committee member that is
            // behind can prove an older version against an older signed root, and the trusted-root ring
            // accepts that by design.
            //
            // An unverified head carries no such guarantee and can be wrong in either direction, so it
            // yields to a proven result, and to time when there is nothing better - which is the only
            // way one recorded above the real version is ever corrected.
            let outranked = entry.verified && !cached_verified;
            let aged_out = !cached_verified && unix_timestamp().saturating_sub(cached_at) > head_ttl.as_secs() as i64;
            // A substate that does not exist ranks below every version, which `Option`'s own ordering
            // gives: nonexistence yields to any head, and any head displaces it.
            if cached_version > version && !outranked && !aged_out {
                return Ok(false);
            }
        }

        let encoded = encode_committed(entry.substate_result)?;

        diesel::insert_into(substate_cache::table)
            .values((
                substate_cache::substate_id.eq(&id),
                substate_cache::version.eq(version),
                substate_cache::verified.eq(entry.verified),
                substate_cache::substate_result.eq(&encoded),
                substate_cache::cached_at.eq(entry.cached_at as i64),
            ))
            .on_conflict(substate_cache::substate_id)
            .do_update()
            .set((
                substate_cache::version.eq(version),
                substate_cache::verified.eq(entry.verified),
                substate_cache::substate_result.eq(&encoded),
                substate_cache::cached_at.eq(entry.cached_at as i64),
            ))
            .execute(self.connection())
            .map_err(|e| StorageError::general(OPERATION, e))?;

        if let (Some(version), Some(proof)) = (version, entry.proof) {
            self.substate_cache_proof_put(&id, version, proof)?;
        }

        Ok(true)
    }

    fn substate_cache_invalidate<I: IntoIterator<Item = SubstateCacheInvalidation>>(
        &mut self,
        invalidations: I,
        state_version: StateVersion,
    ) -> Result<usize, StorageError> {
        let now = unix_timestamp();
        let mut retired = 0;
        for invalidation in invalidations {
            retired += self.apply_substate_cache_invalidation(&invalidation, state_version, now)?;
        }
        Ok(retired)
    }

    fn substate_cache_retire_ahead<I: IntoIterator<Item = (SubstateCacheInvalidation, StateVersion)>>(
        &mut self,
        invalidations: I,
    ) -> Result<usize, StorageError> {
        let now = unix_timestamp();
        let mut retired = 0;
        for (invalidation, state_version) in invalidations {
            retired += self.apply_substate_cache_invalidation(&invalidation, state_version, now)?;
        }
        Ok(retired)
    }

    fn substate_cache_prune(&mut self, journal_retention: Duration, max_entries: usize) -> Result<usize, StorageError> {
        const OPERATION: &str = "substate_cache_prune";
        use crate::storage_sqlite::schema::{substate_cache, substate_cache_invalidations};

        let cutoff = unix_timestamp().saturating_sub(journal_retention.as_secs() as i64);
        diesel::delete(
            substate_cache_invalidations::table.filter(substate_cache_invalidations::invalidated_at.le(cutoff)),
        )
        .execute(self.connection())
        .map_err(|e| StorageError::general(OPERATION, e))?;

        let count: i64 = substate_cache::table
            .count()
            .get_result(self.connection())
            .map_err(|e| StorageError::general(OPERATION, e))?;
        let excess = count.saturating_sub(max_entries as i64);
        let evicted = if excess > 0 {
            // An evicted entry costs one committee round trip to restore, so oldest-written-first is a
            // cheap approximation of least-recently-used: recording a read time would put a write on
            // every cache hit.
            diesel::sql_query(
                "DELETE FROM substate_cache WHERE rowid IN (SELECT rowid FROM substate_cache ORDER BY cached_at ASC \
                 LIMIT ?)",
            )
            .bind::<diesel::sql_types::BigInt, _>(excess)
            .execute(self.connection())
            .map_err(|e| StorageError::general(OPERATION, e))?
        } else {
            0
        };

        // A proof is served only for the version its substate's cached head holds, so one for any
        // other version - superseded, retired by a transition, or evicted - can never be served again.
        diesel::sql_query(
            "DELETE FROM substate_cache_proofs WHERE NOT EXISTS (SELECT 1 FROM substate_cache c WHERE c.substate_id = \
             substate_cache_proofs.substate_id AND c.version = substate_cache_proofs.version)",
        )
        .execute(self.connection())
        .map_err(|e| StorageError::general(OPERATION, e))?;

        Ok(evicted)
    }

    fn upsert_verified_state_root(&mut self, root: &VerifiedStateRoot) -> Result<(), StorageError> {
        const OPERATION: &str = "upsert_verified_state_root";
        // Bounded ring of recent roots retained per (epoch, shard_group): enough to absorb a read
        // landing on a validator a few blocks behind the indexer's last probe, cheap to prune.
        const RING_SIZE: i64 = 16;
        use crate::storage_sqlite::schema::verified_state_roots;

        let epoch = root.epoch.as_u64() as i64;
        let shard_group = root.shard_group.to_parsable_string();

        let inserted = diesel::insert_into(verified_state_roots::table)
            .values(NewVerifiedStateRoot {
                epoch,
                shard_group: shard_group.clone(),
                block_height: root.height.as_u64() as i64,
                block_hash: serialize_hex(root.block_hash),
                state_merkle_root: serialize_hex(root.state_merkle_root),
            })
            .on_conflict((
                verified_state_roots::epoch,
                verified_state_roots::shard_group,
                verified_state_roots::state_merkle_root,
            ))
            .do_nothing()
            .execute(self.connection())
            .map_err(|e| StorageError::general(OPERATION, e))?;

        // The root was already recorded: the ring is unchanged, so skip the prune queries.
        if inserted == 0 {
            return Ok(());
        }

        // Prune everything below the RING_SIZE-th most recent committed height for this key. Committed
        // heights are unique per (epoch, shard_group), so this retains exactly the newest RING_SIZE.
        let kept_heights = verified_state_roots::table
            .select(verified_state_roots::block_height)
            .filter(verified_state_roots::epoch.eq(epoch))
            .filter(verified_state_roots::shard_group.eq(&shard_group))
            .order_by(verified_state_roots::block_height.desc())
            .limit(RING_SIZE)
            .load::<i64>(self.connection())
            .map_err(|e| StorageError::general(OPERATION, e))?;

        if let Some(min_kept_height) = kept_heights.last().copied() {
            diesel::delete(
                verified_state_roots::table
                    .filter(verified_state_roots::epoch.eq(epoch))
                    .filter(verified_state_roots::shard_group.eq(&shard_group))
                    .filter(verified_state_roots::block_height.lt(min_kept_height)),
            )
            .execute(self.connection())
            .map_err(|e| StorageError::general(OPERATION, e))?;
        }

        Ok(())
    }
}

impl<'a> Deref for SqliteStoreWriteTransaction<'a> {
    type Target = SqliteStoreReadTransaction<'a>;

    fn deref(&self) -> &Self::Target {
        self.transaction.as_ref().unwrap()
    }
}

impl DerefMut for SqliteStoreWriteTransaction<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.transaction.as_mut().unwrap()
    }
}

impl Drop for SqliteStoreWriteTransaction<'_> {
    fn drop(&mut self) {
        if self.transaction.is_some() {
            warn!(
                target: LOG_TARGET,
                "Substate store write transaction was not committed/rolled back"
            );
        }
    }
}

impl SqliteStoreWriteTransaction<'_> {
    fn substate_cache_proof_put(
        &mut self,
        id: &str,
        version: i64,
        proof: &SubstateProofData,
    ) -> Result<(), StorageError> {
        const OPERATION: &str = "substate_cache_proof_put";
        use crate::storage_sqlite::schema::substate_cache_proofs;

        let proof_epoch = proof.proof_epoch as i64;
        diesel::insert_into(substate_cache_proofs::table)
            .values((
                substate_cache_proofs::substate_id.eq(id),
                substate_cache_proofs::version.eq(version),
                substate_cache_proofs::value_proof.eq(&proof.substate_value_proof),
                substate_cache_proofs::commit_proof.eq(&proof.commit_proof),
                substate_cache_proofs::proof_epoch.eq(proof_epoch),
            ))
            .on_conflict((substate_cache_proofs::substate_id, substate_cache_proofs::version))
            .do_update()
            .set((
                substate_cache_proofs::value_proof.eq(&proof.substate_value_proof),
                substate_cache_proofs::commit_proof.eq(&proof.commit_proof),
                substate_cache_proofs::proof_epoch.eq(proof_epoch),
            ))
            .execute(self.connection())
            .map_err(|e| StorageError::general(OPERATION, e))?;
        Ok(())
    }

    fn apply_substate_cache_invalidation(
        &mut self,
        invalidation: &SubstateCacheInvalidation,
        state_version: StateVersion,
        now: i64,
    ) -> Result<usize, StorageError> {
        const OPERATION: &str = "substate_cache_invalidate";
        use crate::storage_sqlite::schema::{substate_cache, substate_cache_invalidations};

        let id = invalidation.substate_id().to_string();
        let mut retired = 0;

        if let Some(retires_up_to) = invalidation.retires_up_to() {
            retired += diesel::delete(
                substate_cache::table
                    .filter(substate_cache::substate_id.eq(&id))
                    .filter(substate_cache::version.le(retires_up_to.as_u64() as i64)),
            )
            .execute(self.connection())
            .map_err(|e| StorageError::general(OPERATION, e))?;
        }

        if invalidation.retires_nonexistence() {
            retired += diesel::delete(
                substate_cache::table
                    .filter(substate_cache::substate_id.eq(&id))
                    .filter(substate_cache::version.is_null()),
            )
            .execute(self.connection())
            .map_err(|e| StorageError::general(OPERATION, e))?;
        }

        diesel::insert_into(substate_cache_invalidations::table)
            .values((
                substate_cache_invalidations::substate_id.eq(&id),
                substate_cache_invalidations::state_version.eq(state_version.as_u64() as i64),
                substate_cache_invalidations::substate_version.eq(invalidation.observed_version().as_u64() as i64),
                substate_cache_invalidations::spent.eq(invalidation.is_observed_version_spent()),
                substate_cache_invalidations::invalidated_at.eq(now),
            ))
            .on_conflict(substate_cache_invalidations::substate_id)
            .do_update()
            .set((
                substate_cache_invalidations::state_version.eq(state_version.as_u64() as i64),
                // A floor only ever rises. The transitions of one batch arrive in no stated order,
                // and a destroy of the version below a creation must not lower what the creation
                // showed.
                substate_cache_invalidations::substate_version.eq(diesel::dsl::sql::<diesel::sql_types::BigInt>(
                    "max(substate_version, excluded.substate_version)",
                )),
                // The provenance belongs to whichever version the floor ends up holding, so it
                // follows that same comparison rather than being overwritten. At the one version
                // both a creation and a destroy can show, a spend stands: it is the later of the
                // two whatever order they arrive in.
                substate_cache_invalidations::spent.eq(diesel::dsl::sql::<diesel::sql_types::Bool>(
                    "case when excluded.substate_version > substate_version then excluded.spent when \
                     excluded.substate_version = substate_version then (spent or excluded.spent) else spent end",
                )),
                substate_cache_invalidations::invalidated_at.eq(now),
            ))
            .execute(self.connection())
            .map_err(|e| StorageError::general(OPERATION, e))?;

        Ok(retired)
    }
}
