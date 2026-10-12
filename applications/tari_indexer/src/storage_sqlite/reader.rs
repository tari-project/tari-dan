//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashMap, fmt::Write, str::FromStr};

use diesel::{
    EscapeExpressionMethods,
    ExpressionMethods,
    JoinOnDsl,
    NullableExpressionMethods,
    OptionalExtension,
    QueryDsl,
    RunQueryDsl,
    SqliteConnection,
    TextExpressionMethods,
    dsl,
    sql_types,
};
use serde::de::DeserializeOwned;
use tari_common_types::types::FixedHash;
use tari_engine_types::{
    Utxo,
    events::Event,
    substate::{Substate, SubstateId, SubstateValue},
    transaction_receipt::TransactionReceipt,
};
use tari_indexer_client::types::{
    ListSubstateItem,
    NonFungibleSubstate,
    TransactionEntry,
    TransactionResultSummary,
    TransactionSource,
    UtxoStateUpdateSet,
};
use tari_indexer_lib::substate_cache::SubstateCacheEntry;
use tari_ootle_common_types::{
    Epoch,
    NodeHeight,
    ShardGroup,
    StateVersion,
    SubstateVersion,
    displayable::Displayable,
    shard::Shard,
    substate_type::SubstateType,
};
use tari_ootle_storage::{Ordering, StorageError, consensus_models::EpochCheckpoint, time::PrimitiveDateTime};
use tari_ootle_storage_sqlite::SqliteTransaction;
use tari_ootle_transaction::{Transaction, TransactionId};
use tari_template_lib_types::{
    Amount,
    Hash32,
    ResourceAddress,
    TemplateAddress,
    TransactionReceiptAddress,
    UtxoId,
    crypto::{RistrettoPublicKeyBytes, UtxoTag},
};
use tari_validator_node_rpc::client::SubstateProofData;

use crate::{
    network_state_sync::EventFilter,
    storage_sqlite::{
        models,
        models::{
            EventRecord,
            KeyValue,
            SubstateCacheRow,
            SubstateRecord,
            TemplateCatalogueEntry,
            TemplateCatalogueRow,
            VerifiedStateRoot,
            VerifiedStateRootRow,
            WatchedSubstateEntry,
            WatchedSubstateRow,
        },
        serialization::{deserialize_cbor, deserialize_hex_try_from, deserialize_json, serialize_hex},
    },
    store::{EventQuery, EventsPage, IndexerStoreReadTransaction, TransactionRejectionStatus},
    substate_manager::SubstateResponse,
};

/// Denormalized (outcome, total_fees_paid, created_at) columns left-joined from transaction_receipts.
type ReceiptSummaryRow = Option<(String, i64, PrimitiveDateTime)>;

fn map_receipt_summary(
    (outcome, total_fees_paid, finalized_at): (String, i64, PrimitiveDateTime),
) -> Result<TransactionResultSummary, StorageError> {
    Ok(TransactionResultSummary {
        outcome: outcome.parse().map_err(|_| StorageError::DataInconsistency {
            details: format!("Invalid finalize outcome '{outcome}' in transaction_receipts"),
        })?,
        total_fees_paid: total_fees_paid as u64,
        finalized_at,
    })
}

fn parse_transaction_source(source: &str) -> Result<TransactionSource, StorageError> {
    source.parse().map_err(|_| StorageError::DataInconsistency {
        details: format!("Invalid source '{source}' in transactions"),
    })
}

pub struct SqliteStoreReadTransaction<'a> {
    pub(super) transaction: SqliteTransaction<&'a mut SqliteConnection>,
}

impl<'a> SqliteStoreReadTransaction<'a> {
    pub(super) fn new(transaction: SqliteTransaction<&'a mut SqliteConnection>) -> Self {
        Self { transaction }
    }

    pub(super) fn connection(&mut self) -> &mut SqliteConnection {
        self.transaction.connection()
    }
}

#[derive(Debug, Clone, Copy)]
enum EventRange {
    /// Ids below the bound (all ids when `None`), newest first.
    Before(Option<i64>),
    /// Ids above the bound, oldest first.
    After(i64),
}

impl SqliteStoreReadTransaction<'_> {
    fn query_events(
        &mut self,
        query: &EventQuery,
        range: EventRange,
        offset: u32,
        limit: u32,
    ) -> Result<EventsPage, StorageError> {
        const OPERATION: &str = "query_events";
        use crate::storage_sqlite::schema::events;

        let query_err = |e: diesel::result::Error| StorageError::QueryError {
            reason: format!("{OPERATION}: {e}"),
        };
        let limit = usize::try_from(limit).unwrap_or(usize::MAX);

        let Some(pattern) = query.wildcard_topic() else {
            let rows = filtered_events(query, range)
                .offset(i64::from(offset))
                .limit(limit as i64)
                .load::<EventRecord>(self.connection())
                .map_err(query_err)?;
            let next_cursor = rows.last().filter(|_| rows.len() == limit).map(|row| row.id);
            let events = rows.into_iter().map(decode_event_row).collect::<Result<_, _>>()?;
            return Ok(EventsPage { events, next_cursor });
        };

        let scan_limit = usize::try_from(query.wildcard_scan_limit).unwrap_or(usize::MAX);
        let window = filtered_events(query, range)
            .select((events::id, events::topic))
            .limit(scan_limit as i64)
            .load::<(i64, String)>(self.connection())
            .map_err(query_err)?;
        let matched = window
            .iter()
            .filter(|(_, topic)| EventFilter::topic_matches(pattern, topic))
            .map(|(id, _)| *id)
            .skip(offset as usize)
            .take(limit)
            .collect::<Vec<_>>();
        let next_cursor = if matched.len() == limit {
            matched.last().copied()
        } else if window.len() == scan_limit {
            window.last().map(|(id, _)| *id)
        } else {
            None
        };

        let rows = events::table.filter(events::id.eq_any(&matched)).into_boxed();
        let rows = match range {
            EventRange::Before(_) => rows.order(events::id.desc()),
            EventRange::After(_) => rows.order(events::id.asc()),
        };
        let events = rows
            .load::<EventRecord>(self.connection())
            .map_err(query_err)?
            .into_iter()
            .map(decode_event_row)
            .collect::<Result<_, _>>()?;
        Ok(EventsPage { events, next_cursor })
    }
}

/// The events matching `query`'s indexed filters within `range`, ordered in the range's
/// direction. A wildcard topic is left for the caller to match.
fn filtered_events<'a>(
    query: &EventQuery,
    range: EventRange,
) -> crate::storage_sqlite::schema::events::BoxedQuery<'a, diesel::sqlite::Sqlite> {
    use crate::storage_sqlite::schema::events;

    let mut rows = events::table.into_boxed();
    if let Some(topic) = query.topic.as_deref().filter(|t| !t.contains('*')) {
        rows = rows.filter(events::topic.eq(topic.to_string()));
    }
    if let Some(substate_id) = &query.substate_id {
        rows = rows.filter(events::substate_id.eq(substate_id.to_string()));
    }
    if let Some(template_address) = &query.template_address {
        rows = rows.filter(events::template_address.eq(template_address.to_string()));
    }
    if let Some(resource_address) = &query.resource_address {
        rows = rows.filter(events::resource_address.eq(resource_address.to_string()));
    }
    match range {
        EventRange::Before(Some(before_id)) => rows.filter(events::id.lt(before_id)).order(events::id.desc()),
        EventRange::Before(None) => rows.order(events::id.desc()),
        EventRange::After(after_id) => rows.filter(events::id.gt(after_id)).order(events::id.asc()),
    }
}

fn decode_event_row(row: EventRecord) -> Result<(i64, TransactionId, Event), StorageError> {
    let substate_id = row
        .substate_id
        .as_deref()
        .map(SubstateId::from_str)
        .transpose()
        .map_err(|e| StorageError::DataInconsistency {
            details: format!(
                "Invalid substate_id {} in events table: {}",
                row.substate_id.display(),
                e
            ),
        })?;
    let template_address = Hash32::from_hex(&row.template_address).map_err(|e| StorageError::DataInconsistency {
        details: format!(
            "Invalid template_address {} in events table: {}",
            row.template_address, e
        ),
    })?;
    let tx_hash = Hash32::from_hex(&row.tx_hash).map_err(|e| StorageError::DataInconsistency {
        details: format!("Invalid tx_hash {} in events table: {}", row.tx_hash, e),
    })?;
    let payload = deserialize_json(&row.payload)?;
    Ok((
        row.id,
        TransactionId::from(tx_hash),
        Event::new(substate_id, template_address, row.topic, payload),
    ))
}

impl IndexerStoreReadTransaction for SqliteStoreReadTransaction<'_> {
    fn list_substates(
        &mut self,
        by_id: Option<&SubstateId>,
        by_type: Option<SubstateType>,
        by_template_address: Option<TemplateAddress>,
        limit: Option<u64>,
        offset: Option<u64>,
    ) -> Result<Vec<ListSubstateItem>, StorageError> {
        use crate::storage_sqlite::schema::substates;

        let mut query = substates::table.into_boxed();

        if let Some(substate_id) = by_id {
            query = query.filter(substates::address.eq(substate_id.to_string()));
        }

        if let Some(template_address) = by_template_address {
            query = query.filter(substates::template_address.eq(template_address.to_string()));
        }

        if let Some(substate_type) = by_type {
            let address_like = format!("{}_%", substate_type.as_prefix_str());
            query = query.filter(substates::address.like(address_like));
        }

        if let Some(limit) = limit {
            query = query.limit(limit as i64);
        }
        if let Some(offset) = offset {
            query = query.offset(offset as i64);
        }

        let substates = query
            .order_by(substates::id.desc())
            .load_iter::<SubstateRecord, _>(self.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("list_substates: {}", e),
            })?;

        let items = substates
            .into_iter()
            .map(|res| {
                let s = res.map_err(|e| StorageError::QueryError {
                    reason: format!("list_substates: {}", e),
                })?;
                let substate_id = SubstateId::from_str(&s.address).map_err(|e| StorageError::DataInconsistency {
                    details: format!("Invalid substate address {}: {}", s.address, e),
                })?;
                let version = s.version as u64;
                let template_address = s.template_address.map(|h| deserialize_hex_try_from(&h)).transpose()?;
                let timestamp = s.updated_at;
                Ok(ListSubstateItem {
                    substate_id,
                    module_name: s.module_name,
                    version: SubstateVersion::new(version),
                    template_address,
                    timestamp,
                })
            })
            .collect::<Result<_, StorageError>>()
            .map_err(|e| StorageError::QueryError {
                reason: format!("list_substates: invalid substate items: {}", e),
            })?;

        Ok(items)
    }

    fn get_substate(
        &mut self,
        address: &SubstateId,
        version: Option<SubstateVersion>,
    ) -> Result<Option<SubstateRecord>, StorageError> {
        use crate::storage_sqlite::schema::substates;

        let mut substate_query = substates::table
            .into_boxed()
            .filter(substates::address.eq(address.to_string()));
        if let Some(version) = version {
            substate_query = substate_query.filter(substates::version.eq(version.as_u64() as i64));
        } else {
            substate_query = substate_query.order_by(substates::version.desc())
        }

        substate_query
            .limit(1)
            .first(self.connection())
            .optional()
            .map_err(|e| StorageError::QueryError {
                reason: format!("get_substate: {}", e),
            })
    }

    fn get_substates(&mut self, ids: &[SubstateId]) -> Result<HashMap<SubstateId, Substate>, StorageError> {
        use crate::storage_sqlite::schema::substates;

        let str_ids = ids.iter().map(|id| id.to_string());

        let rows = substates::table
            .select(substates::all_columns)
            .filter(substates::address.eq_any(str_ids))
            .load_iter::<SubstateRecord, _>(self.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("get_substates: {}", e),
            })?;

        rows.into_iter()
            .map(|res| {
                res.map_err(|e| StorageError::QueryError {
                    reason: format!("get_substates: {e}"),
                })
                .and_then(SubstateResponse::try_from)
            })
            .map(|res| {
                res.map(|substate_resp| {
                    (
                        substate_resp.id,
                        Substate::new(substate_resp.version, substate_resp.substate),
                    )
                })
            })
            .collect()
    }

    fn get_non_fungibles_by_resource_address(
        &mut self,
        resource_address: ResourceAddress,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<NonFungibleSubstate>, StorageError> {
        use crate::storage_sqlite::schema::substates;

        let res = substates::table
            .filter(substates::address.like(format!("nft_{}_%", resource_address.as_object_key())))
            .limit(limit as i64)
            .offset(offset as i64)
            .load_iter::<SubstateRecord, _>(self.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("get_non_fungibles_by_resource_address: {}", e),
            })?;

        res.into_iter()
            .map(|res| {
                let row = res.map_err(|e| StorageError::QueryError {
                    reason: format!("get_non_fungibles_by_resource_address: {}", e),
                })?;
                let value: SubstateValue =
                    serde_json::from_str(&row.data).map_err(|e| StorageError::DataInconsistency {
                        details: format!("Failed to parse substate data: {}", e),
                    })?;

                Ok(NonFungibleSubstate {
                    address: row.address.parse().map_err(|e| StorageError::DataInconsistency {
                        details: format!("Failed to parse address: {}", e),
                    })?,
                    version: u64::try_from(row.version).map(SubstateVersion::new).map_err(|e| {
                        StorageError::DataInconsistency {
                            details: format!("Version overflow {}", e),
                        }
                    })?,
                    substate: value,
                })
            })
            .collect()
    }

    fn get_events(
        &mut self,
        query: &EventQuery,
        before_id: Option<i64>,
        offset: u32,
        limit: u32,
    ) -> Result<EventsPage, StorageError> {
        self.query_events(query, EventRange::Before(before_id), offset, limit)
    }

    fn get_events_after_id(
        &mut self,
        query: &EventQuery,
        after_id: i64,
        limit: u32,
    ) -> Result<EventsPage, StorageError> {
        self.query_events(query, EventRange::After(after_id), 0, limit)
    }

    fn list_recent_transactions(
        &mut self,
        last_transaction_id: Option<TransactionId>,
        limit: usize,
        source: Option<TransactionSource>,
    ) -> Result<Vec<TransactionEntry>, StorageError> {
        use crate::storage_sqlite::schema::{transaction_receipts, transactions};

        let start_id = match last_transaction_id {
            // The cursor row can be pruned between two pages of a backwards walk, in which case
            // there is nothing older left to return.
            Some(last_id) => {
                let Some(id) = transactions::table
                    .select(transactions::id)
                    .filter(transactions::transaction_id.eq(serialize_hex(last_id)))
                    .first::<i32>(self.connection())
                    .optional()
                    .map_err(|e| StorageError::QueryError {
                        reason: format!("list_recent_transactions: {e}"),
                    })?
                else {
                    return Ok(vec![]);
                };
                id
            },
            None => i32::MAX,
        };

        let mut query = transactions::table
            .left_join(transaction_receipts::table.on(transaction_receipts::address.eq(transactions::transaction_id)))
            .select((
                transactions::body,
                transactions::created_at,
                transactions::rejected_reason,
                transactions::source,
                (
                    transaction_receipts::outcome,
                    transaction_receipts::total_fees_paid,
                    transaction_receipts::created_at,
                )
                    .nullable(),
            ))
            .filter(transactions::id.lt(start_id))
            .into_boxed();

        if let Some(source) = source {
            query = query.filter(transactions::source.eq(source.as_str()));
        }

        let rows = query
            .order_by(transactions::id.desc())
            .limit(limit as i64)
            .load_iter(self.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("list_recent_transactions: {}", e),
            })?;

        rows.map(|r| {
            r.map_err(|e| StorageError::QueryError {
                reason: format!("list_recent_transactions: {}", e),
            })
            .and_then(
                |(body, created_at, rejected_reason, source, receipt): (
                    String,
                    PrimitiveDateTime,
                    Option<String>,
                    String,
                    ReceiptSummaryRow,
                )| {
                    let full: Transaction = deserialize_json(&body)?;
                    let transaction_id = full.calculate_id();
                    Ok(TransactionEntry {
                        transaction_id,
                        created_at,
                        transaction: full.into(),
                        summary: receipt.map(map_receipt_summary).transpose()?,
                        rejected_reason,
                        source: parse_transaction_source(&source)?,
                    })
                },
            )
        })
        .collect()
    }

    fn get_transaction(&mut self, transaction_id: TransactionId) -> Result<Option<TransactionEntry>, StorageError> {
        use crate::storage_sqlite::schema::{transaction_receipts, transactions};

        let row = transactions::table
            .left_join(transaction_receipts::table.on(transaction_receipts::address.eq(transactions::transaction_id)))
            .select((
                transactions::body,
                transactions::created_at,
                transactions::rejected_reason,
                transactions::source,
                (
                    transaction_receipts::outcome,
                    transaction_receipts::total_fees_paid,
                    transaction_receipts::created_at,
                )
                    .nullable(),
            ))
            .filter(transactions::transaction_id.eq(serialize_hex(transaction_id)))
            .first::<(String, PrimitiveDateTime, Option<String>, String, ReceiptSummaryRow)>(self.connection())
            .optional()
            .map_err(|e| StorageError::QueryError {
                reason: format!("get_transaction: {e}"),
            })?;

        row.map(|(body, created_at, rejected_reason, source, receipt)| {
            let full: Transaction = deserialize_json(&body)?;
            Ok(TransactionEntry {
                // The row was looked up by this id, so it matches full.calculate_id() without recomputing it.
                transaction_id,
                created_at,
                transaction: full.into(),
                summary: receipt.map(map_receipt_summary).transpose()?,
                rejected_reason,
                source: parse_transaction_source(&source)?,
            })
        })
        .transpose()
    }

    fn get_transaction_rejection_status(
        &mut self,
        transaction_id: TransactionId,
    ) -> Result<TransactionRejectionStatus, StorageError> {
        use crate::storage_sqlite::schema::transactions;

        let row = transactions::table
            .select((transactions::rejected_reason, transactions::rejected_at))
            .filter(transactions::transaction_id.eq(serialize_hex(transaction_id)))
            .first::<(Option<String>, Option<PrimitiveDateTime>)>(self.connection())
            .optional()
            .map_err(|e| StorageError::QueryError {
                reason: format!("get_transaction_rejection_status: {e}"),
            })?;

        let Some((reason, rejected_at)) = row else {
            return Ok(TransactionRejectionStatus::NotStored);
        };

        match reason.zip(rejected_at) {
            Some((details, rejected_at)) => Ok(TransactionRejectionStatus::Rejected { details, rejected_at }),
            None => Ok(TransactionRejectionStatus::NotRejected),
        }
    }

    fn list_transaction_receipts(
        &mut self,
        last_id: Option<TransactionReceiptAddress>,
        limit: u64,
        ordering: Ordering,
    ) -> Result<Vec<(TransactionReceiptAddress, TransactionReceipt)>, StorageError> {
        const OPERATION: &str = "list_transaction_receipts";
        use crate::storage_sqlite::schema::transaction_receipts;

        let mut query = transaction_receipts::table
            .select((transaction_receipts::address, transaction_receipts::data))
            .into_boxed();
        if let Some(last_id) = last_id {
            // A cursor this indexer does not store - pruned by retention, or never indexed - is
            // `NotFound`, so the client restarts the listing: no later call can advance past it.
            let cursor = transaction_receipts::table
                .select(transaction_receipts::id)
                .filter(transaction_receipts::address.eq(serialize_hex(last_id.as_object_key())))
                .first::<i32>(self.connection())
                .optional()
                .map_err(|e| StorageError::QueryError {
                    reason: format!("{OPERATION}: {}", e),
                })?
                .ok_or_else(|| StorageError::NotFound {
                    item: "transaction_receipt",
                    key: last_id.to_string(),
                })?;

            query = match ordering {
                Ordering::Ascending => query.filter(transaction_receipts::id.gt(cursor)),
                Ordering::Descending => query.filter(transaction_receipts::id.lt(cursor)),
            }
        }

        query = match ordering {
            Ordering::Ascending => query.order_by(transaction_receipts::id.asc()),
            Ordering::Descending => query.order_by(transaction_receipts::id.desc()),
        };

        let rows = query
            .limit(limit as i64)
            .load_iter::<(String, String), _>(self.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("{OPERATION}: {}", e),
            })?;

        rows.into_iter()
            .map(|row| {
                row.map_err(|e| StorageError::QueryError {
                    reason: format!("{OPERATION}: {}", e),
                })
                .and_then(|(addr, data)| {
                    Ok((
                        TransactionReceiptAddress::from_str(&addr).map_err(|e| StorageError::DataInconsistency {
                            details: format!("Invalid transaction receipt address {}: {}", addr, e),
                        })?,
                        deserialize_json(data)?,
                    ))
                })
            })
            .collect()
    }

    fn get_transaction_receipt(
        &mut self,
        address: &TransactionReceiptAddress,
    ) -> Result<TransactionReceipt, StorageError> {
        const OPERATION: &str = "get_transaction_receipt";
        use crate::storage_sqlite::schema::transaction_receipts;

        let receipt_addr_hex = serialize_hex(address.as_object_key());

        let receipt_entry = transaction_receipts::table
            .select(transaction_receipts::data)
            .filter(transaction_receipts::address.eq(receipt_addr_hex))
            .first::<String>(self.connection())
            .optional()
            .map_err(|e| StorageError::QueryError {
                reason: format!("{OPERATION}: {}", e),
            })?
            .ok_or_else(|| StorageError::NotFound {
                item: "transaction_receipt",
                key: address.to_string(),
            })?;

        deserialize_json(&receipt_entry)
    }

    fn sum_validator_fee_pool_balances(&mut self) -> Result<Amount, StorageError> {
        const OPERATION: &str = "sum_validator_fee_pool_balances";
        use crate::storage_sqlite::schema::substates;

        let total = substates::table
            .select(dsl::sql::<sql_types::Nullable<sql_types::BigInt>>(
                "SUM(json_extract(data, '$.ValidatorFeePool.amount'))",
            ))
            .filter(substates::address.like(format!("{}_%", SubstateType::ValidatorFeePool.as_prefix_str())))
            .get_result::<Option<i64>>(self.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("{OPERATION}: {}", e),
            })?
            .unwrap_or(0);
        u64::try_from(total)
            .map(Amount::from)
            .map_err(|_| StorageError::DataInconsistency {
                details: format!("{OPERATION}: negative fee pool balance {total}"),
            })
    }

    // -------------------------------- KeyValues -------------------------------- //
    fn key_value_get_value<K: AsRef<str>, T: DeserializeOwned>(&mut self, key: K) -> Result<T, StorageError> {
        let key_value = self.key_value_get_raw(key)?;
        deserialize_json(&key_value.value)
    }

    fn key_value_get_raw<K: AsRef<str>>(&mut self, key: K) -> Result<KeyValue<String>, StorageError> {
        const OPERATION: &str = "key_value_get_json";
        use crate::storage_sqlite::schema::key_values;

        let key_value = key_values::table
            .filter(key_values::key.eq(key.as_ref()))
            .first::<models::KeyValueEntry>(self.connection())
            .optional()
            .map_err(|e| StorageError::general(OPERATION, e))?
            .ok_or_else(|| StorageError::NotFound {
                item: "key_value",
                key: key.as_ref().to_string(),
            })?;

        Ok(key_value.into())
    }

    fn epoch_checkpoint_exists(&mut self, shard_group: ShardGroup, epoch: Epoch) -> Result<bool, StorageError> {
        const OPERATION: &str = "epoch_checkpoint_exists";
        use crate::storage_sqlite::schema::epoch_checkpoints;
        let count: i64 = epoch_checkpoints::table
            .filter(epoch_checkpoints::shard_group.eq(shard_group.to_parsable_string()))
            .filter(epoch_checkpoints::epoch.eq(epoch.as_u64() as i64))
            .limit(1)
            .count()
            .get_result(self.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("{OPERATION}: {}", e),
            })?;
        Ok(count > 0)
    }

    fn epoch_checkpoint_get_all(
        &mut self,
        from_epoch: Epoch,
        limit: u64,
    ) -> Result<Vec<EpochCheckpoint>, StorageError> {
        const OPERATION: &str = "epoch_checkpoint_get_all";
        use crate::storage_sqlite::schema::epoch_checkpoints;

        let rows: Vec<String> = epoch_checkpoints::table
            .select(epoch_checkpoints::json_data)
            .filter(epoch_checkpoints::epoch.ge(from_epoch.as_u64() as i64))
            .order_by(epoch_checkpoints::epoch.asc())
            .limit(limit as i64)
            .load(self.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("{OPERATION}: {}", e),
            })?;

        rows.into_iter().map(|json_data| deserialize_json(&json_data)).collect()
    }

    fn epoch_checkpoint_get_latest(&mut self) -> Result<EpochCheckpoint, StorageError> {
        const OPERATION: &str = "epoch_checkpoint_get_latest";
        use crate::storage_sqlite::schema::epoch_checkpoints;

        let json_data: String = epoch_checkpoints::table
            .select(epoch_checkpoints::json_data)
            .order_by(epoch_checkpoints::epoch.desc())
            .first(self.connection())
            .optional()
            .map_err(|e| StorageError::QueryError {
                reason: format!("{OPERATION}: {}", e),
            })?
            .ok_or_else(|| StorageError::NotFound {
                item: "EpochCheckpoint",
                key: "latest".to_string(),
            })?;

        deserialize_json(&json_data)
    }

    fn utxos_get_max_state_version(
        &mut self,
        resource_address: ResourceAddress,
        shard: Shard,
    ) -> Result<StateVersion, StorageError> {
        const OPERATION: &str = "utxos_get_max_state_version";
        use crate::storage_sqlite::schema::utxos;
        let resource_address = resource_address.to_string();
        let shard = shard.as_u32() as i32;
        // State versions are stored `as i64`, so those from 2^63 up read back negative and are the
        // highest when there are any. Each half is a seek on the (resource_address, state_version)
        // index.
        let mut max_version = None;
        for above_i64_max in [true, false] {
            let mut query = utxos::table
                .select(dsl::max(utxos::state_version))
                .filter(utxos::resource_address.eq(&resource_address))
                .filter(utxos::shard.eq(shard))
                .into_boxed();
            if above_i64_max {
                query = query.filter(utxos::state_version.lt(0));
            }
            max_version = query
                .get_result::<Option<i64>>(self.connection())
                .map_err(|e| StorageError::QueryError {
                    reason: format!("{OPERATION}: {}", e),
                })?;
            if max_version.is_some() {
                break;
            }
        }

        Ok(max_version.map_or_else(StateVersion::zero, |v| StateVersion::new(v as u64)))
    }

    fn utxos_get_updates(
        &mut self,
        resource_address: ResourceAddress,
        from_epoch: Epoch,
        shard: Shard,
        from_state_version: StateVersion,
        unspent_only: bool,
        limit: u32,
    ) -> Result<UtxoStateUpdateSet, StorageError> {
        const OPERATION: &str = "get_utxo_updates";
        use crate::storage_sqlite::schema::utxos;

        // One state version covers a whole synced batch, so every UTXO touched by the same block on
        // this shard shares it. `state_version` is therefore not a unique key and the caller's
        // cursor cannot address a row within a version — it can only say "everything after version
        // V". A response that ends mid-version would strand the rest of that version above the
        // cursor the caller then resumes from, so a version is either wholly included or wholly
        // absent. One row beyond the limit is read to find where that boundary falls.
        let overshoot_limit = i64::from(limit).saturating_add(1);
        let base_query = || {
            let mut query = utxos::table
                .filter(utxos::resource_address.eq(resource_address.to_string()))
                .filter(utxos::epoch.ge(from_epoch.as_u64() as i64))
                .filter(utxos::shard.eq(shard.as_u32() as i32))
                .order_by(utxos::state_version.asc())
                .into_boxed();
            if unspent_only {
                // Only return unspent UTXOs
                query = query
                    .filter(utxos::is_spent.eq(false))
                    .filter(utxos::is_burnt.eq(false));
            }
            query
        };
        // Versions from 2^63 up are stored negative and order above every non-negative one. The read
        // is split at that boundary so that each half is an ordered range on the
        // (resource_address, state_version) index, stopping at the limit.
        let from = from_state_version.as_u64() as i64;
        let mut rows = if from >= 0 {
            base_query()
                .filter(utxos::state_version.gt(from))
                .limit(overshoot_limit)
                .load::<models::UtxoRecord>(self.connection())
        } else {
            base_query()
                .filter(utxos::state_version.gt(from))
                .filter(utxos::state_version.lt(0))
                .limit(overshoot_limit)
                .load::<models::UtxoRecord>(self.connection())
        }
        .map_err(|e| StorageError::QueryError {
            reason: format!("{OPERATION}: {}", e),
        })?;
        let remaining = overshoot_limit - rows.len() as i64;
        if from >= 0 && remaining > 0 {
            rows.extend(
                base_query()
                    .filter(utxos::state_version.lt(0))
                    .limit(remaining)
                    .load::<models::UtxoRecord>(self.connection())
                    .map_err(|e| StorageError::QueryError {
                        reason: format!("{OPERATION}: {}", e),
                    })?,
            );
        }

        let has_more = rows.len() as i64 == overshoot_limit;
        let mut converted = Vec::with_capacity(rows.len());
        let mut max_epoch = Epoch::zero();
        for row in rows {
            let epoch = Epoch(row.epoch as u64);
            let (state_version, update) = row.try_convert_to_update()?;
            max_epoch = max_epoch.max(epoch);
            converted.push((state_version, update));
        }

        if has_more {
            // Rows are ordered by state version, so the overshoot row's version is the highest
            // present and is the only one that can be partial.
            let partial_version = converted
                .last()
                .map(|(state_version, _)| *state_version)
                .expect("has_more implies at least one row");
            let complete = converted.partition_point(|(v, _)| *v < partial_version);
            if complete > 0 {
                converted.truncate(complete);
            } else {
                // The whole read sits at one version, so there is no complete earlier version to
                // stop at. Returning nothing would leave the caller's cursor where it was and no
                // way past this shard, so the version is served whole and the limit is overrun.
                let mut whole_version = utxos::table
                    .filter(utxos::resource_address.eq(resource_address.to_string()))
                    .filter(utxos::state_version.eq(partial_version.as_u64() as i64))
                    .filter(utxos::epoch.ge(from_epoch.as_u64() as i64))
                    .filter(utxos::shard.eq(shard.as_u32() as i32))
                    .into_boxed();
                if unspent_only {
                    whole_version = whole_version
                        .filter(utxos::is_spent.eq(false))
                        .filter(utxos::is_burnt.eq(false));
                }
                let rows = whole_version
                    .load::<models::UtxoRecord>(self.connection())
                    .map_err(|e| StorageError::QueryError {
                        reason: format!("{OPERATION}: {}", e),
                    })?;

                converted.clear();
                for row in rows {
                    let epoch = Epoch(row.epoch as u64);
                    let (state_version, update) = row.try_convert_to_update()?;
                    max_epoch = max_epoch.max(epoch);
                    converted.push((state_version, update));
                }
            }
        }

        let max_state_version = converted
            .last()
            .map_or_else(StateVersion::zero, |(state_version, _)| *state_version);

        Ok(UtxoStateUpdateSet {
            updates: converted.into_iter().map(|(_, update)| update).collect(),
            max_state_version,
            max_epoch,
            has_more,
        })
    }

    fn utxos_list(
        &mut self,
        resource_address: &ResourceAddress,
        from_id: Option<UtxoId>,
        limit: u32,
    ) -> Result<Vec<(UtxoId, Utxo)>, StorageError> {
        const OPERATION: &str = "utxos_list";
        use crate::storage_sqlite::schema::utxos;

        let mut query = utxos::table
            .filter(utxos::resource_address.eq(resource_address.to_string()))
            .filter(utxos::is_spent.eq(false))
            .filter(utxos::is_burnt.eq(false))
            .into_boxed();

        if let Some(from_id) = from_id {
            let uxo = alias!(utxos as uxo);
            let subquery = uxo
                .select(uxo.field(utxos::id))
                .filter(uxo.field(utxos::commitment).eq(from_id.to_commitment_hex_string()))
                .limit(1)
                .single_value()
                .assume_not_null();
            query = query.filter(utxos::id.gt(subquery));
        }

        let rows = query
            .limit(i64::from(limit))
            .order_by(utxos::id.asc())
            .load_iter::<models::UtxoRecord, _>(self.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("{OPERATION}: {}", e),
            })?;

        rows.map(|res| {
            res.map_err(|e| StorageError::QueryError {
                reason: format!("{OPERATION}: {}", e),
            })
            .and_then(|row| {
                let (address, utxo) = row.try_convert_to_utxo()?;
                Ok((*address.id(), utxo))
            })
        })
        .collect()
    }

    fn utxos_get_unspent_by_public_nonce_and_tag(
        &mut self,
        resource_address: &ResourceAddress,
        tag_and_nonce_pairs: &[(UtxoTag, RistrettoPublicKeyBytes)],
    ) -> Result<Vec<(UtxoId, Utxo)>, StorageError> {
        const OPERATION: &str = "utxos_get_unspent_by_public_nonce_and_tag";
        use crate::storage_sqlite::schema::utxos;

        // Diesel does not support tuple IN queries, so we have to use raw SQL here (https://github.com/diesel-rs/diesel/issues/3222#issuecomment-2088239186)
        let mut tag_and_nonce_query = String::with_capacity(29 + tag_and_nonce_pairs.len() * 50);
        tag_and_nonce_query.push_str("(utxo_tag,public_nonce) IN (");
        for (i, (tag, nonce)) in tag_and_nonce_pairs.iter().enumerate() {
            // SQLINJECTION not possible as we control the types
            write!(
                &mut tag_and_nonce_query,
                "({},'{}')",
                tag.value() as i32,
                serialize_hex(nonce)
            )
            .expect("write to string is infallible");

            if i < tag_and_nonce_pairs.len() - 1 {
                tag_and_nonce_query.push(',');
            }
        }
        tag_and_nonce_query.push(')');

        let rows = utxos::table
            .filter(utxos::resource_address.eq(resource_address.to_string()))
            .filter(dsl::sql::<sql_types::Bool>(&tag_and_nonce_query))
            .filter(utxos::is_spent.eq(false))
            .filter(utxos::is_burnt.eq(false))
            .load_iter::<models::UtxoRecord, _>(self.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("{OPERATION}: {}", e),
            })?;
        let mut utxos = Vec::new();
        for row in rows {
            let row = row.map_err(|e| StorageError::QueryError {
                reason: format!("{OPERATION}: {}", e),
            })?;

            let (address, utxo) = row.try_convert_to_utxo()?;
            utxos.push((*address.id(), utxo));
        }

        Ok(utxos)
    }

    fn list_template_catalogue(
        &mut self,
        name_filter: Option<&str>,
        after: Option<&TemplateAddress>,
        limit: u64,
    ) -> Result<Vec<TemplateCatalogueEntry>, StorageError> {
        const OPERATION: &str = "list_template_catalogue";
        use crate::storage_sqlite::schema::template_catalogue;

        let mut query = template_catalogue::table
            .order_by(template_catalogue::id.asc())
            .into_boxed();

        if let Some(name) = name_filter {
            let escaped = name.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
            query = query.filter(
                template_catalogue::template_name
                    .like(format!("%{escaped}%"))
                    .escape('\\'),
            );
        }

        if let Some(after_address) = after {
            let after_hex = hex::encode(after_address.as_slice());
            let cursor_id = template_catalogue::table
                .select(template_catalogue::id)
                .filter(template_catalogue::template_address.eq(&after_hex))
                .first::<i32>(self.connection())
                .optional()
                .map_err(|e| StorageError::QueryError {
                    reason: format!("{OPERATION}: cursor lookup: {e}"),
                })?
                .ok_or_else(|| StorageError::QueryError {
                    reason: format!("{OPERATION}: cursor template address {after_address} not found"),
                })?;

            query = query.filter(template_catalogue::id.gt(cursor_id));
        }

        let rows = query
            .limit(limit as i64)
            .load::<TemplateCatalogueRow>(self.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("{OPERATION}: {e}"),
            })?;

        rows.into_iter()
            .map(|row| {
                TemplateCatalogueEntry::try_from(row).map_err(|e| StorageError::DecodingError {
                    operation: OPERATION,
                    item: "TemplateCatalogueEntry",
                    details: e.to_string(),
                })
            })
            .collect()
    }

    fn get_template_catalogue_entry(
        &mut self,
        template_address: &TemplateAddress,
    ) -> Result<TemplateCatalogueEntry, StorageError> {
        const OPERATION: &str = "get_template_catalogue_entry";
        use crate::storage_sqlite::schema::template_catalogue;

        let addr_hex = hex::encode(template_address.as_slice());
        let row = template_catalogue::table
            .filter(template_catalogue::template_address.eq(&addr_hex))
            .first::<TemplateCatalogueRow>(self.connection())
            .optional()
            .map_err(|e| StorageError::QueryError {
                reason: format!("{OPERATION}: {e}"),
            })?
            .ok_or_else(|| StorageError::NotFound {
                item: "template_catalogue_entry",
                key: template_address.to_string(),
            })?;

        TemplateCatalogueEntry::try_from(row).map_err(|e| StorageError::DecodingError {
            operation: OPERATION,
            item: "TemplateCatalogueEntry",
            details: e.to_string(),
        })
    }

    fn list_watched_substates(
        &mut self,
        template_address: Option<&TemplateAddress>,
        limit: u64,
        offset: u64,
    ) -> Result<Vec<WatchedSubstateEntry>, StorageError> {
        const OPERATION: &str = "list_watched_substates";
        use crate::storage_sqlite::schema::watched_substates;

        let mut query = watched_substates::table.into_boxed();

        if let Some(template_address) = template_address {
            query = query.filter(watched_substates::template_address.eq(template_address.to_string()));
        }

        let rows = query
            .limit(limit as i64)
            .offset(offset as i64)
            .load::<WatchedSubstateRow>(self.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("{OPERATION}: {e}"),
            })?;

        rows.into_iter()
            .map(|row| {
                let component_address =
                    SubstateId::from_str(&row.component_address).map_err(|e| StorageError::DataInconsistency {
                        details: format!("{OPERATION}: Invalid component address {}: {e}", row.component_address),
                    })?;
                let template_address =
                    TemplateAddress::from_hex(&row.template_address).map_err(|e| StorageError::DataInconsistency {
                        details: format!("{OPERATION}: Invalid template address {}: {e}", row.template_address),
                    })?;
                Ok(WatchedSubstateEntry {
                    component_address,
                    template_address,
                })
            })
            .collect()
    }

    fn is_state_root_trusted(
        &mut self,
        epoch: Epoch,
        shard_group: ShardGroup,
        root: &FixedHash,
    ) -> Result<bool, StorageError> {
        const OPERATION: &str = "is_state_root_trusted";
        use crate::storage_sqlite::schema::verified_state_roots;
        let count: i64 = verified_state_roots::table
            .filter(verified_state_roots::epoch.eq(epoch.as_u64() as i64))
            .filter(verified_state_roots::shard_group.eq(shard_group.to_parsable_string()))
            .filter(verified_state_roots::state_merkle_root.eq(serialize_hex(root)))
            .limit(1)
            .count()
            .get_result(self.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("{OPERATION}: {e}"),
            })?;
        Ok(count > 0)
    }

    fn get_latest_verified_state_root(
        &mut self,
        epoch: Epoch,
        shard_group: ShardGroup,
    ) -> Result<Option<VerifiedStateRoot>, StorageError> {
        const OPERATION: &str = "get_latest_verified_state_root";
        use crate::storage_sqlite::schema::verified_state_roots;
        let row: Option<VerifiedStateRootRow> = verified_state_roots::table
            .filter(verified_state_roots::epoch.eq(epoch.as_u64() as i64))
            .filter(verified_state_roots::shard_group.eq(shard_group.to_parsable_string()))
            .order_by(verified_state_roots::block_height.desc())
            .first(self.connection())
            .optional()
            .map_err(|e| StorageError::QueryError {
                reason: format!("{OPERATION}: {e}"),
            })?;
        row.map(|row| {
            Ok(VerifiedStateRoot {
                epoch,
                shard_group,
                height: NodeHeight(row.block_height as u64),
                block_hash: deserialize_hex_try_from(&row.block_hash)?,
                state_merkle_root: deserialize_hex_try_from(&row.state_merkle_root)?,
            })
        })
        .transpose()
    }

    fn substate_cache_get(&mut self, substate_id: &SubstateId) -> Result<Option<SubstateCacheEntry>, StorageError> {
        const OPERATION: &str = "substate_cache_get";
        use crate::storage_sqlite::schema::substate_cache;

        let row: Option<SubstateCacheRow> = substate_cache::table
            .filter(substate_cache::substate_id.eq(substate_id.to_string()))
            .first(self.connection())
            .optional()
            .map_err(|e| StorageError::QueryError {
                reason: format!("{OPERATION}: {e}"),
            })?;

        row.map(|row| {
            Ok(SubstateCacheEntry {
                version: row.version.map(|v| SubstateVersion::new(v as u64)),
                substate_result: deserialize_cbor(&row.substate_result)?,
                cached_at: row.cached_at as u64,
                verified: row.verified,
            })
        })
        .transpose()
    }

    fn substate_cache_proof_get(
        &mut self,
        substate_id: &SubstateId,
        version: SubstateVersion,
    ) -> Result<Option<SubstateProofData>, StorageError> {
        const OPERATION: &str = "substate_cache_proof_get";
        use crate::storage_sqlite::schema::substate_cache_proofs;

        let row: Option<(Vec<u8>, Vec<u8>, i64)> = substate_cache_proofs::table
            .select((
                substate_cache_proofs::value_proof,
                substate_cache_proofs::commit_proof,
                substate_cache_proofs::proof_epoch,
            ))
            .filter(substate_cache_proofs::substate_id.eq(substate_id.to_string()))
            .filter(substate_cache_proofs::version.eq(version.as_u64() as i64))
            .first(self.connection())
            .optional()
            .map_err(|e| StorageError::QueryError {
                reason: format!("{OPERATION}: {e}"),
            })?;

        Ok(row.map(|(value_proof, commit_proof, proof_epoch)| SubstateProofData {
            substate_value_proof: value_proof,
            commit_proof,
            proof_epoch: proof_epoch as u64,
            substate_down_proof: None,
            destroyed_at_state_version: None,
        }))
    }
}
