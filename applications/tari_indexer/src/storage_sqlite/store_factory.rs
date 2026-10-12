//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{fmt::Debug, fs::create_dir_all, path::PathBuf, sync::Arc, time::Duration};

use async_trait::async_trait;
use deadpool_diesel::{
    Runtime,
    sqlite::{Hook, HookError, Manager, Pool},
};
use diesel::{Connection, RunQueryDsl, SqliteConnection, sql_query};
use diesel_migrations::{EmbeddedMigrations, MigrationHarness};
use tari_ootle_storage::StorageError;
use tari_ootle_storage_sqlite::{SqliteTransaction, error::SqliteStorageError};
use tokio::sync::Mutex;

#[cfg(feature = "metrics")]
use crate::storage_sqlite::metrics::{StorageFileStats, StorageMetrics};
use crate::{
    storage_sqlite::{reader::SqliteStoreReadTransaction, writer::SqliteStoreWriteTransaction},
    store::{IndexerStore, IndexerStoreReader, IndexerStoreWriteTransaction},
};

const LOG_TARGET: &str = "tari::indexer::storage_sqlite";
const POOL_MAX_SIZE: usize = 16;
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
const MIGRATIONS: EmbeddedMigrations = embed_migrations!("./src/storage_sqlite/migrations");

#[derive(Clone)]
pub struct SqliteIndexerStore {
    pool: Pool,
    /// Admits one writer at a time, in arrival order. SQLite allows a single writer, and its busy
    /// handler retries on a sleep-and-poll backoff with no queue, so under a burst of writers some
    /// lose every retry until `BUSY_TIMEOUT` expires. Queuing here leaves `BUSY_TIMEOUT` to cover
    /// only locks taken outside this store, such as a WAL checkpoint.
    write_lock: Arc<Mutex<()>>,
    path: PathBuf,
    #[cfg(feature = "metrics")]
    metrics: Option<StorageMetrics>,
}

impl SqliteIndexerStore {
    pub fn try_create(path: PathBuf) -> Result<Self, StorageError> {
        create_dir_all(path.parent().unwrap()).map_err(|_| StorageError::FileSystemPathDoesNotExist)?;

        let database_url = path.to_str().expect("database_url utf-8 error").to_string();
        let db_path = path.clone();

        // Run migrations on a one-shot connection before opening the pool, so pooled connections
        // never observe a partially-migrated schema.
        let mut migration_conn = SqliteConnection::establish(&database_url).map_err(SqliteStorageError::from)?;
        apply_pragmas(&mut migration_conn).map_err(|source| SqliteStorageError::DieselError {
            source,
            operation: "set pragma",
        })?;
        if let Err(err) = migration_conn.run_pending_migrations(MIGRATIONS) {
            log::error!(target: LOG_TARGET, "Error running migrations: {}", err);
        }
        drop(migration_conn);

        let manager = Manager::new(database_url, Runtime::Tokio1);
        let pool = Pool::builder(manager)
            .max_size(POOL_MAX_SIZE)
            .post_create(Hook::async_fn(|conn, _metrics| {
                Box::pin(async move {
                    conn.interact(apply_pragmas)
                        .await
                        .map_err(|e| HookError::message(format!("post_create panicked: {e}")))?
                        .map_err(|e| HookError::message(format!("apply_pragmas failed: {e}")))?;
                    Ok(())
                })
            }))
            .build()
            .map_err(|e| StorageError::General {
                details: format!("Failed to build sqlite connection pool: {}", e),
            })?;

        Ok(Self {
            pool,
            write_lock: Arc::new(Mutex::new(())),
            path: db_path,
            #[cfg(feature = "metrics")]
            metrics: None,
        })
    }

    #[cfg(feature = "metrics")]
    pub fn with_metrics(mut self, metrics: StorageMetrics) -> Self {
        self.metrics = Some(metrics);
        self
    }

    async fn acquire(&self) -> Result<deadpool_diesel::sqlite::Connection, StorageError> {
        #[cfg(feature = "metrics")]
        let started = std::time::Instant::now();
        let conn = self.pool.get().await.map_err(|e| StorageError::General {
            details: format!("Failed to acquire sqlite connection from pool: {}", e),
        })?;
        #[cfg(feature = "metrics")]
        if let Some(metrics) = &self.metrics {
            metrics.observe_connection_wait(started.elapsed());
        }
        Ok(conn)
    }

    /// Sizes of the database and its WAL, after a passive checkpoint has copied back every WAL frame
    /// that no open read transaction still needs. The checkpoint is the one SQLite runs on its own
    /// once the WAL passes its autocheckpoint size; running it here makes the frames it leaves behind
    /// a measure of how far open reads are holding the WAL back.
    #[cfg(feature = "metrics")]
    pub(super) async fn file_stats(&self) -> Result<StorageFileStats, StorageError> {
        use diesel::{QueryableByName, sql_types::BigInt};

        #[derive(QueryableByName)]
        struct WalCheckpoint {
            #[diesel(sql_type = BigInt)]
            log: i64,
            #[diesel(sql_type = BigInt)]
            checkpointed: i64,
        }
        #[derive(QueryableByName)]
        struct PageSize {
            #[diesel(sql_type = BigInt)]
            page_size: i64,
        }
        #[derive(QueryableByName)]
        struct FreelistCount {
            #[diesel(sql_type = BigInt)]
            freelist_count: i64,
        }

        let conn = self.acquire().await?;
        let (checkpoint, page_size, freelist) = conn
            .interact(|c| -> Result<_, diesel::result::Error> {
                let checkpoint = sql_query("PRAGMA wal_checkpoint(PASSIVE);").get_result::<WalCheckpoint>(c)?;
                let page_size = sql_query("PRAGMA page_size;").get_result::<PageSize>(c)?;
                let freelist = sql_query("PRAGMA freelist_count;").get_result::<FreelistCount>(c)?;
                Ok((checkpoint, page_size, freelist))
            })
            .await
            .map_err(|e| StorageError::General {
                details: format!("Pool interact panicked: {}", e),
            })?
            .map_err(|e| StorageError::general("file_stats", e))?;

        let mut wal_path = self.path.clone().into_os_string();
        wal_path.push("-wal");
        let file_len = |path: &std::path::Path| std::fs::metadata(path).map_or(0, |m| m.len());
        let non_negative = |n: i64| u64::try_from(n).unwrap_or(0);

        Ok(StorageFileStats {
            db_bytes: file_len(&self.path),
            wal_bytes: file_len(PathBuf::from(wal_path).as_path()),
            wal_uncheckpointed_frames: non_negative(checkpoint.log - checkpoint.checkpointed),
            freelist_bytes: non_negative(freelist.freelist_count).saturating_mul(non_negative(page_size.page_size)),
        })
    }
}

impl Debug for SqliteIndexerStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqliteIndexerStore")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl IndexerStoreReader for SqliteIndexerStore {
    type ReadTransaction<'a> = SqliteStoreReadTransaction<'a>;

    async fn with_read_tx<F, R, E>(&self, f: F) -> Result<R, E>
    where
        F: for<'a> FnOnce(&mut Self::ReadTransaction<'a>) -> Result<R, E> + Send + 'static,
        R: Send + 'static,
        E: From<StorageError> + Send + 'static,
    {
        let conn = self.acquire().await?;
        let result: Result<R, E> = conn
            .interact(move |c| -> Result<R, E> {
                let inner = SqliteTransaction::begin(c)
                    .map_err(StorageError::from)
                    .map_err(E::from)?;
                let mut tx = SqliteStoreReadTransaction::new(inner);
                f(&mut tx)
            })
            .await
            .map_err(|e| StorageError::General {
                details: format!("Pool interact panicked: {}", e),
            })?;
        result
    }
}

#[async_trait]
impl IndexerStore for SqliteIndexerStore {
    type WriteTransaction<'a> = SqliteStoreWriteTransaction<'a>;

    async fn with_write_tx<F, R, E>(&self, f: F) -> Result<R, E>
    where
        F: for<'a> FnOnce(&mut Self::WriteTransaction<'a>) -> Result<R, E> + Send + 'static,
        R: Send + 'static,
        E: From<StorageError> + Send + 'static,
    {
        #[cfg(feature = "metrics")]
        let lock_requested = std::time::Instant::now();
        // Taken before a pooled connection so that queued writers do not hold connections readers
        // need. The guard moves into the closure because `interact` keeps running on its blocking
        // thread if this future is dropped, and the lock must be held until the transaction ends.
        let write_guard = self.write_lock.clone().lock_owned().await;
        let conn = self.acquire().await?;
        #[cfg(feature = "metrics")]
        let metrics = self.metrics.clone();
        let result: Result<R, E> = conn
            .interact(move |c| -> Result<R, E> {
                let _write_guard = write_guard;
                let inner = match SqliteTransaction::begin_immediate(c) {
                    Ok(inner) => inner,
                    Err(err) => {
                        #[cfg(feature = "metrics")]
                        if let Some(metrics) = &metrics &&
                            is_busy(&err)
                        {
                            metrics.inc_write_lock_busy();
                        }
                        return Err(E::from(StorageError::from(err)));
                    },
                };
                #[cfg(feature = "metrics")]
                let lock_acquired = std::time::Instant::now();
                #[cfg(feature = "metrics")]
                if let Some(metrics) = &metrics {
                    metrics.observe_write_lock_wait(lock_acquired - lock_requested);
                }

                let mut tx = SqliteStoreWriteTransaction::new(inner);
                let result = match f(&mut tx) {
                    Ok(r) => tx.commit().map(|_| r).map_err(E::from),
                    Err(e) => {
                        if let Err(err) = tx.rollback() {
                            log::error!(target: LOG_TARGET, "Failed to rollback transaction: {}", err);
                        }
                        Err(e)
                    },
                };

                #[cfg(feature = "metrics")]
                if let Some(metrics) = &metrics {
                    metrics.observe_write_lock_hold(lock_acquired.elapsed());
                }
                result
            })
            .await
            .map_err(|e| StorageError::General {
                details: format!("Pool interact panicked: {}", e),
            })?;
        result
    }
}

/// True if `err` is SQLite giving up on a lock after the busy timeout (`SQLITE_BUSY`).
#[cfg(feature = "metrics")]
fn is_busy(err: &SqliteStorageError) -> bool {
    matches!(
        err,
        SqliteStorageError::DieselError {
            source: diesel::result::Error::DatabaseError(_, info),
            ..
        } if info.message().contains("database is locked")
    )
}

fn apply_pragmas(conn: &mut SqliteConnection) -> Result<(), diesel::result::Error> {
    let busy_timeout_ms = BUSY_TIMEOUT.as_millis();
    sql_query("PRAGMA journal_mode = WAL;").execute(conn)?;
    sql_query("PRAGMA synchronous = NORMAL;").execute(conn)?;
    sql_query("PRAGMA foreign_keys = ON;").execute(conn)?;
    sql_query(format!("PRAGMA busy_timeout = {};", busy_timeout_ms)).execute(conn)?;
    Ok(())
}

/// Appends one event per topic to the store at `db_path`, taking the next ids in the order given.
#[cfg(test)]
pub(crate) fn insert_test_events(db_path: &std::path::Path, topics: &[&str]) {
    use tari_common_types::types::FixedHash;

    use crate::storage_sqlite::{models::NewEvent, schema::events, serialization::serialize_json};

    let mut conn = SqliteConnection::establish(db_path.to_str().unwrap()).unwrap();
    let tx_hash = FixedHash::zero().to_string();
    let payload = serialize_json(&tari_template_lib_types::Metadata::new()).unwrap();
    for topic in topics {
        diesel::insert_into(events::table)
            .values(NewEvent {
                template_address: FixedHash::zero().to_string(),
                tx_hash: &tx_hash,
                topic,
                payload: payload.clone(),
                substate_id: None,
                resource_address: None,
                epoch: 0,
            })
            .execute(&mut conn)
            .unwrap();
    }
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use tari_common_types::types::FixedHash;
    use tari_engine_types::{
        fees::FeeReceiptBuilder,
        substate::SubstateId,
        transaction_receipt::{FinalizeOutcome, TransactionReceipt},
    };
    use tari_indexer_client::types::TransactionSource;
    use tari_indexer_lib::substate_cache::{FetchWatermark, SubstateCacheEntry, SubstateCacheEntryRef};
    use tari_ootle_common_types::{Epoch, NodeHeight, ShardGroup, StateVersion, SubstateVersion};
    use tari_ootle_transaction::{Transaction, TransactionId};
    use tari_validator_node_rpc::client::SubstateResult;

    use super::*;
    use crate::{
        storage_sqlite::models::{SubstateCacheInvalidation, VerifiedStateRoot},
        store::{IndexerStoreReadTransaction, IndexerStoreReader, IndexerStoreWriteTransaction},
    };

    /// Well above every `max_epoch` the tests use, so the clamp is inert unless a test is about it.
    const RETENTION_CEILING: Epoch = Epoch(1_000_000);

    fn shard_group() -> ShardGroup {
        ShardGroup::new_checked(1, 4).unwrap()
    }

    fn tip_at(height: u64) -> VerifiedStateRoot {
        // Each height gets a distinct root, as committed heights do on a real chain.
        VerifiedStateRoot {
            epoch: Epoch(1),
            shard_group: shard_group(),
            height: NodeHeight(height),
            block_hash: FixedHash::new([height as u8; 32]),
            state_merkle_root: FixedHash::new([height as u8; 32]),
        }
    }

    /// One state version covers a whole synced batch, so a shard's UTXOs cluster into version
    /// groups. These build a shard whose groups straddle the read limit.
    fn utxo_resource() -> tari_template_lib_types::ResourceAddress {
        use std::str::FromStr;
        tari_template_lib_types::ResourceAddress::from_str(
            "resource_0000000000000000000000000000000000000000000000000000000000000000",
        )
        .unwrap()
    }

    fn unspent_at(seq: u8, state_version: u64) -> crate::storage_sqlite::models::UtxoUpdateRecord {
        use tari_engine_types::{UtxoOutput, crypto::OutputBody};
        use tari_template_lib_types::{
            EncryptedData,
            UtxoId,
            crypto::{RistrettoPublicKeyBytes, UtxoTag},
            stealth::SpendAuthorization,
        };

        use crate::storage_sqlite::models::{UtxoUnspent, UtxoUpdateRecord};

        let address = tari_template_lib_types::UtxoAddress::new(utxo_resource(), UtxoId::from_array([seq; 32]));
        UtxoUpdateRecord::Unspent(Box::new(UtxoUnspent {
            address,
            version: SubstateVersion::ZERO,
            shard: tari_ootle_common_types::shard::Shard::from(1u32),
            state_version: StateVersion::new(state_version),
            utxo_output: UtxoOutput {
                output: OutputBody {
                    public_nonce: RistrettoPublicKeyBytes::from_bytes(&[seq; 32]).unwrap(),
                    encrypted_data: EncryptedData::empty(),
                    minimum_value_promise: 0,
                    viewable_balance: None,
                },
                auth: SpendAuthorization::Key(RistrettoPublicKeyBytes::from_bytes(&[seq; 32]).unwrap()),
                tag: UtxoTag::from(0u32),
            },
            is_frozen: false,
        }))
    }

    async fn store_with_utxos(groups: &[(u64, u8)]) -> (tempfile::TempDir, SqliteIndexerStore) {
        let (dir, store) = temp_store().await;
        let mut seq = 0u8;
        let mut records = Vec::new();
        for &(state_version, count) in groups {
            for _ in 0..count {
                seq += 1;
                records.push(unspent_at(seq, state_version));
            }
        }
        store
            .with_write_tx(move |tx| tx.batch_insert_utxo_updates(Epoch(1), records))
            .await
            .unwrap();
        (dir, store)
    }

    async fn read_updates(
        store: &SqliteIndexerStore,
        from: u64,
        limit: u32,
    ) -> tari_indexer_client::types::UtxoStateUpdateSet {
        store
            .with_read_tx(move |tx| {
                tx.utxos_get_updates(
                    utxo_resource(),
                    Epoch(0),
                    tari_ootle_common_types::shard::Shard::from(1u32),
                    StateVersion::new(from),
                    false,
                    limit,
                )
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn a_read_stops_on_a_version_boundary_not_mid_version() {
        // Limit 4 falls inside the 3-row group at version 20.
        let (_dir, store) = store_with_utxos(&[(10, 2), (20, 3), (30, 1)]).await;

        let set = read_updates(&store, 0, 4).await;

        assert!(set.has_more);
        // Version 20 is held back whole rather than half-served.
        assert_eq!(set.max_state_version, StateVersion::new(10));
        assert_eq!(set.updates.len(), 2);
    }

    #[tokio::test]
    async fn resuming_from_the_reported_version_loses_no_update() {
        let (_dir, store) = store_with_utxos(&[(10, 2), (20, 3), (30, 1)]).await;

        let mut seen = 0;
        let mut cursor = 0;
        loop {
            let set = read_updates(&store, cursor, 4).await;
            seen += set.updates.len();
            cursor = set.max_state_version.as_u64();
            if !set.has_more {
                break;
            }
        }

        assert_eq!(seen, 6);
    }

    #[tokio::test]
    async fn reads_order_and_resume_across_the_whole_version_range() {
        // Versions from 2^63 up are stored negative in the signed column.
        let high = i64::MAX as u64 + 1;
        let (_dir, store) = store_with_utxos(&[(10, 1), (high, 1), (u64::MAX, 1)]).await;

        let set = read_updates(&store, 0, 1).await;
        assert_eq!(set.max_state_version, StateVersion::new(10));
        assert!(set.has_more);
        let set = read_updates(&store, 10, 1).await;
        assert_eq!(set.max_state_version, StateVersion::new(high));
        assert!(set.has_more);
        let set = read_updates(&store, high, 1).await;
        assert_eq!(set.max_state_version, StateVersion::new(u64::MAX));
        assert!(!set.has_more);
        assert!(read_updates(&store, u64::MAX, 1).await.updates.is_empty());

        // A read whose limit straddles the boundary takes both sides in order.
        let set = read_updates(&store, 0, 2).await;
        assert_eq!(set.updates.len(), 2);
        assert_eq!(set.max_state_version, StateVersion::new(high));
        assert!(set.has_more);
        let set = read_updates(&store, 0, 10).await;
        assert_eq!(set.updates.len(), 3);
        assert_eq!(set.max_state_version, StateVersion::new(u64::MAX));
        assert!(!set.has_more);

        let max = store
            .with_read_tx(|tx| {
                tx.utxos_get_max_state_version(utxo_resource(), tari_ootle_common_types::shard::Shard::from(1u32))
            })
            .await
            .unwrap();
        assert_eq!(max, StateVersion::new(u64::MAX));
    }

    #[tokio::test]
    async fn a_version_wider_than_the_limit_is_served_whole() {
        // No complete earlier version to stop at, so the limit gives way rather than the read
        // returning nothing and stranding the cursor.
        let (_dir, store) = store_with_utxos(&[(10, 5), (20, 1)]).await;

        let set = read_updates(&store, 0, 2).await;

        assert_eq!(set.updates.len(), 5);
        assert_eq!(set.max_state_version, StateVersion::new(10));
        assert!(set.has_more);
    }

    #[tokio::test]
    async fn a_drained_read_reports_no_more() {
        let (_dir, store) = store_with_utxos(&[(10, 2), (20, 1)]).await;

        let set = read_updates(&store, 0, 10).await;

        assert!(!set.has_more);
        assert_eq!(set.updates.len(), 3);
        assert_eq!(set.max_state_version, StateVersion::new(20));
    }

    async fn temp_store() -> (tempfile::TempDir, SqliteIndexerStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteIndexerStore::try_create(dir.path().join("indexer.db")).unwrap();
        (dir, store)
    }

    #[tokio::test]
    async fn tari_economics_round_trips() {
        use tari_template_lib_types::Amount;

        use crate::{
            storage_sqlite::models::Key,
            store::{IndexerStoreWriteTransaction, ReadOnlyStore},
        };

        let (_dir, store) = temp_store().await;
        store
            .with_write_tx(|tx| {
                tx.key_value_set(Key::TariAccumulatedClaimed, Amount::from(1_000u64))?;
                tx.key_value_set(Key::TariAccumulatedExhaustBurn, Amount::from(50u64))?;
                tx.key_value_set(Key::TariAccumulatedFees, Amount::from(800u64))?;
                tx.key_value_set(Key::TariAccumulatedReceiptExhaustBurn, Amount::from(40u64))
            })
            .await
            .unwrap();

        let econ = ReadOnlyStore::new(store.clone()).get_tari_economics().await.unwrap();
        assert_eq!(econ.total_claimed, Amount::from(1_000u64));
        assert_eq!(econ.total_exhaust_burned, Amount::from(50u64));
        assert_eq!(econ.fee_volume, Amount::from(800u64));
        assert_eq!(econ.receipt_exhaust_burned, Amount::from(40u64));
    }

    #[tokio::test]
    async fn tari_economics_defaults_to_zero_when_unset() {
        use tari_template_lib_types::Amount;

        use crate::store::ReadOnlyStore;

        let (_dir, store) = temp_store().await;
        let econ = ReadOnlyStore::new(store.clone()).get_tari_economics().await.unwrap();
        assert_eq!(econ.total_claimed, Amount::zero());
        assert_eq!(econ.fee_volume, Amount::zero());
        assert_eq!(econ.receipt_exhaust_burned, Amount::zero());
    }

    #[tokio::test]
    async fn validator_claimable_fees_sums_the_latest_pool_balances() {
        use tari_engine_types::{ValidatorFeePool, substate::SubstateValue};
        use tari_ootle_storage::consensus_models::SubstateData;
        use tari_template_lib_types::{Amount, ValidatorFeePoolAddress, crypto::RistrettoPublicKeyBytes};

        use crate::store::ReadOnlyStore;

        fn pool(n: u8, version: u64, amount: u64) -> SubstateData {
            SubstateData {
                substate_id: SubstateId::ValidatorFeePool(ValidatorFeePoolAddress::from_array([n; 32])),
                version: SubstateVersion::new(version),
                value: SubstateValue::from(ValidatorFeePool::new(RistrettoPublicKeyBytes::default(), amount)).into(),
                template_metadata: None,
            }
        }

        let (_dir, store) = temp_store().await;
        let econ = ReadOnlyStore::new(store.clone()).get_tari_economics().await.unwrap();
        assert_eq!(econ.validator_claimable_fees, Amount::zero());

        store
            .with_write_tx(|tx| {
                tx.upsert_substate(&pool(1, 0, 300))?;
                tx.upsert_substate(&pool(2, 0, 200))?;
                // A claim of 250 from the first pool.
                tx.upsert_substate(&pool(1, 1, 50))
            })
            .await
            .unwrap();

        let econ = ReadOnlyStore::new(store.clone()).get_tari_economics().await.unwrap();
        assert_eq!(econ.validator_claimable_fees, Amount::from(250u64));
    }

    #[tokio::test]
    async fn tari_total_supply_nets_receipt_burn_not_header() {
        use tari_template_lib_types::Amount;

        use crate::{
            storage_sqlite::models::Key,
            store::{IndexerStoreWriteTransaction, ReadOnlyStore},
        };

        let (_dir, store) = temp_store().await;
        store
            .with_write_tx(|tx| {
                tx.key_value_set(Key::TariAccumulatedClaimed, Amount::from(1_000u64))?;
                // Deliberately different from the receipt burn to prove supply ignores the header total.
                tx.key_value_set(Key::TariAccumulatedExhaustBurn, Amount::from(999u64))?;
                tx.key_value_set(Key::TariAccumulatedReceiptExhaustBurn, Amount::from(40u64))
            })
            .await
            .unwrap();

        let supply = ReadOnlyStore::new(store.clone()).get_tari_total_supply().await.unwrap();
        assert_eq!(supply, Amount::from(960u64));
    }

    #[tokio::test]
    async fn verified_state_roots_ring_prunes_to_sixteen() {
        let (_dir, store) = temp_store().await;

        // Record 19 distinct committed heights for the same (epoch, shard group).
        for h in 1..=19u64 {
            let root = tip_at(h);
            store
                .with_write_tx(move |tx| tx.upsert_verified_state_root(&root))
                .await
                .unwrap();
        }

        // The latest reflects the most recent committed height.
        let latest = store
            .with_read_tx(move |tx| tx.get_latest_verified_state_root(Epoch(1), shard_group()))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(latest.height, NodeHeight(19));

        // The newest 16 (heights 4..=19) remain trusted; the oldest 3 (1..=3) were pruned.
        for h in 4..=19u64 {
            let root = FixedHash::new([h as u8; 32]);
            assert!(
                store
                    .with_read_tx(move |tx| tx.is_state_root_trusted(Epoch(1), shard_group(), &root))
                    .await
                    .unwrap(),
                "height {h} should still be trusted"
            );
        }
        for h in 1..=3u64 {
            let root = FixedHash::new([h as u8; 32]);
            assert!(
                !store
                    .with_read_tx(move |tx| tx.is_state_root_trusted(Epoch(1), shard_group(), &root))
                    .await
                    .unwrap(),
                "height {h} should have been pruned"
            );
        }
    }

    #[tokio::test]
    async fn verified_state_roots_upsert_is_idempotent() {
        let (_dir, store) = temp_store().await;
        for _ in 0..3 {
            let root = tip_at(5);
            store
                .with_write_tx(move |tx| tx.upsert_verified_state_root(&root))
                .await
                .unwrap();
        }
        let hash = FixedHash::new([5u8; 32]);
        assert!(
            store
                .with_read_tx(move |tx| tx.is_state_root_trusted(Epoch(1), shard_group(), &hash))
                .await
                .unwrap()
        );
        // An unrecorded root is not trusted.
        let other = FixedHash::new([99u8; 32]);
        assert!(
            !store
                .with_read_tx(move |tx| tx.is_state_root_trusted(Epoch(1), shard_group(), &other))
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn recent_transactions_include_receipt_summary() {
        use tari_common_types::types::PrivateKey;
        use tari_engine_types::{
            fees::FeeReceiptBuilder,
            transaction_receipt::{FinalizeOutcome, TransactionReceipt},
        };
        use tari_ootle_transaction::Transaction;

        use crate::store::IndexerStoreWriteTransaction;

        let (_dir, store) = temp_store().await;

        let transaction = Transaction::builder_localnet(Epoch(1)).build_and_seal(&PrivateKey::from(123u64));
        let tx_id = transaction.calculate_id();
        store
            .with_write_tx(move |tx| tx.upsert_submitted_transaction(&transaction, RETENTION_CEILING))
            .await
            .unwrap();

        // No receipt indexed yet — the transaction lists without a summary.
        let entries = store
            .with_read_tx(move |tx| tx.list_recent_transactions(None, 10, None))
            .await
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].transaction_id, tx_id);
        assert!(entries[0].summary.is_none());

        let receipt = TransactionReceipt {
            outcome: FinalizeOutcome::FeeIntentCommit,
            diff_summary: Default::default(),
            fee_withdrawals: [].into(),
            events: [].into(),
            fee_receipt: FeeReceiptBuilder::default().with_total_fees_paid(123).build(),
            epoch: Epoch(1),
            intent_commitment: Default::default(),
        };
        store
            .with_write_tx(move |tx| {
                tx.batch_insert_transaction_receipts([(tx_id.into_receipt_address(), receipt)], &[])
            })
            .await
            .unwrap();

        let entries = store
            .with_read_tx(move |tx| tx.list_recent_transactions(None, 10, None))
            .await
            .unwrap();
        let summary = entries[0].summary.as_ref().unwrap();
        assert!(summary.outcome.is_fee_intent_commit());
        assert_eq!(summary.total_fees_paid, 123);

        let entry = store
            .with_read_tx(move |tx| tx.get_transaction(tx_id))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(entry.summary.as_ref().unwrap().total_fees_paid, 123);
    }

    #[tokio::test]
    async fn prune_transactions_removes_only_aged_rows_and_keeps_receipts() {
        let (_dir, store) = temp_store().await;

        // A transaction's retention epoch is its max_epoch until a receipt supplies a commit epoch.
        let ids = insert_transactions(&store, &[Epoch(5), Epoch(6), Epoch(20)]).await;

        // A receipt for a transaction that is about to be pruned: it must survive.
        let receipt_address = ids[0].into_receipt_address();
        store
            .with_write_tx(move |tx| {
                tx.batch_insert_transaction_receipts([(receipt_address, receipt_at(Epoch(5)))], &[])
            })
            .await
            .unwrap();

        let num_pruned = store
            .with_write_tx(move |tx| tx.prune_transactions_before_epoch(Epoch(10), 100))
            .await
            .unwrap();
        assert_eq!(num_pruned, 2);

        let remaining = store
            .with_read_tx(move |tx| tx.list_recent_transactions(None, 10, None))
            .await
            .unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].transaction_id, ids[2]);

        // The receipt of the pruned transaction is untouched.
        store
            .with_read_tx(move |tx| tx.get_transaction_receipt(&receipt_address))
            .await
            .unwrap();
    }

    /// A transaction that never commits gets no receipt — mempool rejection and a consensus abort that
    /// commits nothing both leave one — so its `max_epoch` stays its retention key. Without that
    /// fallback these rows, the ones retention exists to bound, would never age out.
    #[tokio::test]
    async fn a_transaction_that_never_commits_is_retained_on_its_max_epoch() {
        let (_dir, store) = temp_store().await;

        let ids = insert_transactions(&store, &[Epoch(5), Epoch(20)]).await;
        for id in &ids {
            let id = *id;
            store
                .with_write_tx(move |tx| tx.set_transaction_rejected(id, "rejected by mempool validation"))
                .await
                .unwrap();
        }

        let num_pruned = store
            .with_write_tx(move |tx| tx.prune_transactions_before_epoch(Epoch(10), 100))
            .await
            .unwrap();
        assert_eq!(num_pruned, 1);

        let remaining = store
            .with_read_tx(move |tx| tx.list_recent_transactions(None, 10, None))
            .await
            .unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].transaction_id, ids[1]);
    }

    /// A committed transaction is retained from the epoch it committed in, not from the last epoch it
    /// could have been sequenced in — a wide `max_epoch` must not hold its record open.
    #[tokio::test]
    async fn indexing_a_receipt_moves_retention_to_the_commit_epoch() {
        let (_dir, store) = temp_store().await;

        let ids = insert_transactions(&store, &[Epoch(100)]).await;
        let receipt_address = ids[0].into_receipt_address();

        // On its max_epoch alone this transaction is far from the cutoff.
        let num_pruned = store
            .with_write_tx(move |tx| tx.prune_transactions_before_epoch(Epoch(10), 100))
            .await
            .unwrap();
        assert_eq!(num_pruned, 0);

        store
            .with_write_tx(move |tx| {
                tx.batch_insert_transaction_receipts([(receipt_address, receipt_at(Epoch(2)))], &[])
            })
            .await
            .unwrap();

        let num_pruned = store
            .with_write_tx(move |tx| tx.prune_transactions_before_epoch(Epoch(10), 100))
            .await
            .unwrap();
        assert_eq!(num_pruned, 1);
    }

    /// The prune select must be servable from `transactions_retention_epoch_idx`. Ordering it by any
    /// column other than the one it filters on silently degrades it to a full table scan that runs
    /// under the database-wide write lock, including on the common call that prunes nothing.
    #[tokio::test]
    async fn prune_select_is_served_by_the_retention_epoch_index() {
        #[derive(diesel::QueryableByName)]
        struct QueryPlanRow {
            #[diesel(sql_type = diesel::sql_types::Text)]
            detail: String,
        }

        let (_dir, store) = temp_store().await;
        let plan = store
            .with_read_tx(|tx| {
                sql_query(
                    "explain query plan select id from transactions where retention_epoch < 100 order by \
                     retention_epoch asc limit 500",
                )
                .load::<QueryPlanRow>(tx.connection())
                .map_err(|e| StorageError::general("explain query plan", e))
            })
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.detail)
            .collect::<Vec<_>>()
            .join("\n");

        assert!(
            plan.contains("transactions_retention_epoch_idx"),
            "prune select does not use the retention epoch index: {plan}"
        );
        assert!(
            !plan.contains("SCAN transactions"),
            "prune select falls back to a scan: {plan}"
        );
    }

    #[tokio::test]
    async fn rejection_status_distinguishes_a_pruned_row_from_an_unrejected_one() {
        use tari_common_types::types::PrivateKey;
        use tari_ootle_transaction::Transaction;

        use crate::store::{IndexerStoreWriteTransaction, TransactionRejectionStatus};

        let (_dir, store) = temp_store().await;

        let transaction = Transaction::builder_localnet(Epoch(1)).build_and_seal(&PrivateKey::from(7u64));
        let tx_id = transaction.calculate_id();

        // Never submitted here.
        let status = store
            .with_read_tx(move |tx| tx.get_transaction_rejection_status(tx_id))
            .await
            .unwrap();
        assert!(matches!(status, TransactionRejectionStatus::NotStored));

        store
            .with_write_tx(move |tx| tx.upsert_submitted_transaction(&transaction, RETENTION_CEILING))
            .await
            .unwrap();
        let status = store
            .with_read_tx(move |tx| tx.get_transaction_rejection_status(tx_id))
            .await
            .unwrap();
        assert!(matches!(status, TransactionRejectionStatus::NotRejected));

        store
            .with_write_tx(move |tx| tx.set_transaction_rejected(tx_id, "nope"))
            .await
            .unwrap();
        let status = store
            .with_read_tx(move |tx| tx.get_transaction_rejection_status(tx_id))
            .await
            .unwrap();
        assert!(matches!(status, TransactionRejectionStatus::Rejected { details, .. } if details == "nope"));

        // Pruned rows report as unstored, so callers do not re-issue the rejection write forever.
        store
            .with_write_tx(move |tx| tx.prune_transactions_before_epoch(Epoch(10), 100))
            .await
            .unwrap();
        let status = store
            .with_read_tx(move |tx| tx.get_transaction_rejection_status(tx_id))
            .await
            .unwrap();
        assert!(matches!(status, TransactionRejectionStatus::NotStored));
    }

    #[tokio::test]
    async fn recent_transactions_returns_an_empty_page_when_the_cursor_was_pruned() {
        use tari_common_types::types::PrivateKey;
        use tari_ootle_transaction::Transaction;

        use crate::store::IndexerStoreWriteTransaction;

        let (_dir, store) = temp_store().await;

        let transaction = Transaction::builder_localnet(Epoch(1)).build_and_seal(&PrivateKey::from(11u64));
        let cursor = transaction.calculate_id();
        store
            .with_write_tx(move |tx| tx.upsert_submitted_transaction(&transaction, RETENTION_CEILING))
            .await
            .unwrap();

        store
            .with_write_tx(move |tx| tx.prune_transactions_before_epoch(Epoch(10), 100))
            .await
            .unwrap();

        let page = store
            .with_read_tx(move |tx| tx.list_recent_transactions(Some(cursor), 10, None))
            .await
            .unwrap();
        assert!(page.is_empty());
    }
    /// The same transaction reaching the indexer twice — submitted here and gossiped back by the
    /// network, or gossiped twice — must not produce a second row.
    #[tokio::test]
    async fn a_transaction_stored_twice_produces_one_row() {
        use tari_common_types::types::PrivateKey;
        use tari_ootle_transaction::Transaction;

        use crate::store::IndexerStoreWriteTransaction;

        let (_dir, store) = temp_store().await;

        let transaction = Transaction::builder_localnet(Epoch(20)).build_and_seal(&PrivateKey::from(31u64));
        let tx_id = transaction.calculate_id();

        let gossiped = transaction.clone();
        store
            .with_write_tx(move |tx| {
                tx.insert_batch_transactions([&gossiped], TransactionSource::Gossip, RETENTION_CEILING)
            })
            .await
            .unwrap();
        let gossiped = transaction.clone();
        let num_inserted = store
            .with_write_tx(move |tx| {
                tx.insert_batch_transactions([&gossiped], TransactionSource::Gossip, RETENTION_CEILING)
            })
            .await
            .unwrap();
        assert_eq!(num_inserted, 0);
        store
            .with_write_tx(move |tx| tx.upsert_submitted_transaction(&transaction, RETENTION_CEILING))
            .await
            .unwrap();

        let entries = store
            .with_read_tx(move |tx| tx.list_recent_transactions(None, 10, None))
            .await
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].transaction_id, tx_id);
    }

    /// The network gossips a submission straight back, so which write lands first is a race. A direct
    /// submission must claim the row either way, or the recorded source answers "who won a race"
    /// rather than "did this indexer's clients submit this".
    #[tokio::test]
    async fn a_direct_submission_claims_a_row_already_stored_from_gossip() {
        use tari_common_types::types::PrivateKey;
        use tari_ootle_transaction::Transaction;

        use crate::store::IndexerStoreWriteTransaction;

        let (_dir, store) = temp_store().await;

        let transaction = Transaction::builder_localnet(Epoch(20)).build_and_seal(&PrivateKey::from(37u64));
        let tx_id = transaction.calculate_id();

        let gossiped = transaction.clone();
        store
            .with_write_tx(move |tx| {
                tx.insert_batch_transactions([&gossiped], TransactionSource::Gossip, RETENTION_CEILING)
            })
            .await
            .unwrap();
        let entry = store
            .with_read_tx(move |tx| tx.get_transaction(tx_id))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(entry.source, TransactionSource::Gossip);

        let submitted = transaction.clone();
        store
            .with_write_tx(move |tx| tx.upsert_submitted_transaction(&submitted, RETENTION_CEILING))
            .await
            .unwrap();
        let entry = store
            .with_read_tx(move |tx| tx.get_transaction(tx_id))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(entry.source, TransactionSource::Local);

        // The reverse order does not demote it: gossip never overwrites a stored row.
        let gossiped = transaction.clone();
        store
            .with_write_tx(move |tx| {
                tx.insert_batch_transactions([&gossiped], TransactionSource::Gossip, RETENTION_CEILING)
            })
            .await
            .unwrap();
        let entry = store
            .with_read_tx(move |tx| tx.get_transaction(tx_id))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(entry.source, TransactionSource::Local);
    }

    #[tokio::test]
    async fn recent_transactions_filters_by_source() {
        use tari_common_types::types::PrivateKey;
        use tari_ootle_transaction::Transaction;

        use crate::store::IndexerStoreWriteTransaction;

        let (_dir, store) = temp_store().await;

        let submitted = Transaction::builder_localnet(Epoch(20)).build_and_seal(&PrivateKey::from(41u64));
        let submitted_id = submitted.calculate_id();
        let gossiped = Transaction::builder_localnet(Epoch(20)).build_and_seal(&PrivateKey::from(43u64));
        let gossiped_id = gossiped.calculate_id();

        store
            .with_write_tx(move |tx| tx.upsert_submitted_transaction(&submitted, RETENTION_CEILING))
            .await
            .unwrap();
        store
            .with_write_tx(move |tx| {
                tx.insert_batch_transactions([&gossiped], TransactionSource::Gossip, RETENTION_CEILING)
            })
            .await
            .unwrap();

        let all = store
            .with_read_tx(move |tx| tx.list_recent_transactions(None, 10, None))
            .await
            .unwrap();
        assert_eq!(all.len(), 2);

        let local = store
            .with_read_tx(move |tx| tx.list_recent_transactions(None, 10, Some(TransactionSource::Local)))
            .await
            .unwrap();
        assert_eq!(local.len(), 1);
        assert_eq!(local[0].transaction_id, submitted_id);

        let from_gossip = store
            .with_read_tx(move |tx| tx.list_recent_transactions(None, 10, Some(TransactionSource::Gossip)))
            .await
            .unwrap();
        assert_eq!(from_gossip.len(), 1);
        assert_eq!(from_gossip[0].transaction_id, gossiped_id);
    }

    /// A gossiped transaction's retention key is its own `max_epoch`, so pruning must reach it on the
    /// same schedule as a submitted one. An unprunable row is how the gossip firehose would grow the
    /// database without bound.
    #[tokio::test]
    async fn pruning_reaches_gossiped_transactions() {
        use tari_common_types::types::PrivateKey;
        use tari_ootle_transaction::Transaction;

        use crate::store::IndexerStoreWriteTransaction;

        let (_dir, store) = temp_store().await;

        let aged = Transaction::builder_localnet(Epoch(5)).build_and_seal(&PrivateKey::from(47u64));
        let current = Transaction::builder_localnet(Epoch(20)).build_and_seal(&PrivateKey::from(53u64));
        let current_id = current.calculate_id();
        store
            .with_write_tx(move |tx| {
                tx.insert_batch_transactions([&aged, &current], TransactionSource::Gossip, RETENTION_CEILING)
            })
            .await
            .unwrap();

        let num_pruned = store
            .with_write_tx(move |tx| tx.prune_transactions_before_epoch(Epoch(10), 100))
            .await
            .unwrap();
        assert_eq!(num_pruned, 1);

        let remaining = store
            .with_read_tx(move |tx| tx.list_recent_transactions(None, 10, None))
            .await
            .unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].transaction_id, current_id);
    }

    /// `max_epoch` is chosen by whoever authored the transaction and is the retention key until a
    /// receipt supplies a commit epoch, so an unclamped one buys a row the pruner never reaches.
    /// `i64::MAX` is the value that actually does it: the column is a signed SQL integer, so a
    /// larger epoch reinterprets as negative and gets pruned immediately instead.
    #[tokio::test]
    async fn a_distant_max_epoch_is_clamped_to_the_retention_ceiling() {
        use tari_common_types::types::PrivateKey;
        use tari_ootle_transaction::Transaction;

        use crate::store::IndexerStoreWriteTransaction;

        let (_dir, store) = temp_store().await;

        let ceiling = Epoch(500);
        for (i, max_epoch) in [Epoch(i64::MAX as u64), Epoch(u64::MAX), Epoch(100_000)]
            .into_iter()
            .enumerate()
        {
            let transaction = Transaction::builder_localnet(max_epoch).build_and_seal(&PrivateKey::from(i as u64 + 61));
            store
                .with_write_tx(move |tx| {
                    tx.insert_batch_transactions([&transaction], TransactionSource::Gossip, ceiling)
                })
                .await
                .unwrap();
        }

        // Every row sits at the ceiling, so a pruner running one epoch past it reaches all of them.
        let num_pruned = store
            .with_write_tx(move |tx| tx.prune_transactions_before_epoch(Epoch(501), 100))
            .await
            .unwrap();
        assert_eq!(num_pruned, 3);
    }

    /// A transaction still inside the ceiling keeps its own `max_epoch` — the clamp is a cap, not a
    /// flat assignment, or every row would age out together regardless of its real window.
    #[tokio::test]
    async fn the_ceiling_does_not_move_a_transaction_that_is_already_under_it() {
        use tari_common_types::types::PrivateKey;
        use tari_ootle_transaction::Transaction;

        use crate::store::IndexerStoreWriteTransaction;

        let (_dir, store) = temp_store().await;

        let transaction = Transaction::builder_localnet(Epoch(20)).build_and_seal(&PrivateKey::from(71u64));
        store
            .with_write_tx(move |tx| {
                tx.insert_batch_transactions([&transaction], TransactionSource::Gossip, Epoch(500))
            })
            .await
            .unwrap();

        let num_pruned = store
            .with_write_tx(move |tx| tx.prune_transactions_before_epoch(Epoch(21), 100))
            .await
            .unwrap();
        assert_eq!(num_pruned, 1);
    }

    /// The source filter pages backwards by id like the unfiltered listing does. Without a matching
    /// index it walks the whole gossip stream to collect a page of local rows.
    #[tokio::test]
    async fn the_source_filtered_listing_uses_the_source_index() {
        #[derive(diesel::QueryableByName)]
        struct QueryPlanRow {
            #[diesel(sql_type = diesel::sql_types::Text)]
            detail: String,
        }

        let (_dir, store) = temp_store().await;

        let plan = store
            .with_read_tx(|tx| {
                // The projection and the receipts LEFT JOIN are what can push SQLite off the index
                // onto the primary key plus a filter, so the plan has to be taken over the real
                // query rather than a simplified stand-in.
                sql_query(
                    "explain query plan select t.body, t.created_at, t.rejected_reason, t.source, r.outcome, \
                     r.total_fees_paid, r.created_at from transactions t left join transaction_receipts r on \
                     r.address = t.transaction_id where t.id < 100 and t.source = 'local' order by t.id desc limit 10",
                )
                .load::<QueryPlanRow>(tx.connection())
                .map_err(|e| StorageError::general("explain query plan", e))
            })
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.detail)
            .collect::<Vec<_>>()
            .join("\n");

        assert!(
            plan.contains("transactions_source_id_idx"),
            "source filtered listing does not use the source index: {plan}"
        );
    }

    async fn insert_transactions(store: &SqliteIndexerStore, max_epochs: &[Epoch]) -> Vec<TransactionId> {
        use tari_common_types::types::PrivateKey;

        let mut ids = Vec::new();
        for (i, max_epoch) in max_epochs.iter().enumerate() {
            let transaction = Transaction::builder_localnet(*max_epoch).build_and_seal(&PrivateKey::from(i as u64));
            ids.push(transaction.calculate_id());
            store
                .with_write_tx(move |tx| tx.upsert_submitted_transaction(&transaction, RETENTION_CEILING))
                .await
                .unwrap();
        }
        ids
    }

    fn receipt_at(epoch: Epoch) -> TransactionReceipt {
        TransactionReceipt {
            outcome: FinalizeOutcome::FeeIntentCommit,
            diff_summary: Default::default(),
            fee_withdrawals: [].into(),
            events: [].into(),
            fee_receipt: FeeReceiptBuilder::default().with_total_fees_paid(123).build(),
            epoch,
            intent_commitment: Default::default(),
        }
    }

    fn receipt_with_events_at(epoch: Epoch, num_events: usize) -> TransactionReceipt {
        let events = (0..num_events)
            .map(|i| {
                tari_engine_types::events::Event::new(
                    None,
                    Default::default(),
                    format!("test.topic.{i}"),
                    tari_template_lib_types::Metadata::new(),
                )
            })
            .collect::<Vec<_>>();
        TransactionReceipt {
            events: events.into(),
            ..receipt_at(epoch)
        }
    }

    fn receipt_address(n: u8) -> tari_template_lib_types::TransactionReceiptAddress {
        TransactionId::new([n; 32]).into_receipt_address()
    }

    async fn insert_receipts(store: &SqliteIndexerStore, receipts: Vec<(u8, TransactionReceipt)>) {
        store
            .with_write_tx(move |tx| {
                tx.batch_insert_transaction_receipts(
                    receipts.into_iter().map(|(n, receipt)| (receipt_address(n), receipt)),
                    &[],
                )
            })
            .await
            .unwrap();
    }

    #[derive(diesel::QueryableByName)]
    struct EpochRow {
        #[diesel(sql_type = diesel::sql_types::BigInt)]
        epoch: i64,
    }

    async fn event_epochs(store: &SqliteIndexerStore) -> Vec<i64> {
        store
            .with_read_tx(|tx| {
                sql_query("select epoch from events order by id")
                    .load::<EpochRow>(tx.connection())
                    .map_err(|e| StorageError::general("event_epochs", e))
            })
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.epoch)
            .collect()
    }

    async fn query_plan(store: &SqliteIndexerStore, query: &'static str) -> String {
        #[derive(diesel::QueryableByName)]
        struct QueryPlanRow {
            #[diesel(sql_type = diesel::sql_types::Text)]
            detail: String,
        }

        store
            .with_read_tx(move |tx| {
                sql_query(format!("explain query plan {query}"))
                    .load::<QueryPlanRow>(tx.connection())
                    .map_err(|e| StorageError::general("explain query plan", e))
            })
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.detail)
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[tokio::test]
    async fn prune_receipts_removes_only_those_committed_before_the_cutoff() {
        let (_dir, store) = temp_store().await;
        insert_receipts(&store, vec![
            (1, receipt_at(Epoch(5))),
            (2, receipt_at(Epoch(9))),
            (3, receipt_at(Epoch(10))),
        ])
        .await;

        let num_pruned = store
            .with_write_tx(move |tx| tx.prune_transaction_receipts_before_epoch(Epoch(10), 100))
            .await
            .unwrap();
        assert_eq!(num_pruned, 2);

        let remaining = store
            .with_read_tx(|tx| tx.list_transaction_receipts(None, 10, tari_ootle_storage::Ordering::Ascending))
            .await
            .unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].0, receipt_address(3));
    }

    #[tokio::test]
    async fn receipt_listing_resumes_after_the_cursor() {
        let (_dir, store) = temp_store().await;
        insert_receipts(&store, vec![
            (1, receipt_at(Epoch(5))),
            (2, receipt_at(Epoch(6))),
            (3, receipt_at(Epoch(7))),
        ])
        .await;

        let page = store
            .with_read_tx(|tx| {
                tx.list_transaction_receipts(Some(receipt_address(1)), 10, tari_ootle_storage::Ordering::Ascending)
            })
            .await
            .unwrap();
        assert_eq!(page.iter().map(|(addr, _)| *addr).collect::<Vec<_>>(), vec![
            receipt_address(2),
            receipt_address(3)
        ]);
    }

    /// Retention deletes the oldest receipts first, which is where an ascending reader's cursor sits. An
    /// empty page there would read as caught up on every call, so a missing cursor must be reported.
    #[tokio::test]
    async fn a_pruned_receipt_cursor_is_not_found() {
        let (_dir, store) = temp_store().await;
        insert_receipts(&store, vec![(1, receipt_at(Epoch(5))), (2, receipt_at(Epoch(20)))]).await;
        store
            .with_write_tx(move |tx| tx.prune_transaction_receipts_before_epoch(Epoch(10), 100))
            .await
            .unwrap();

        for ordering in [
            tari_ootle_storage::Ordering::Ascending,
            tari_ootle_storage::Ordering::Descending,
        ] {
            let err = store
                .with_read_tx(move |tx| tx.list_transaction_receipts(Some(receipt_address(1)), 10, ordering))
                .await
                .unwrap_err();
            assert!(matches!(err, StorageError::NotFound { .. }), "{err}");
        }
    }

    /// The receipt count is network history, reported next to the other accumulated totals. Pruning
    /// local storage must not rewind it.
    #[tokio::test]
    async fn the_receipt_count_includes_pruned_receipts() {
        use crate::store::ReadOnlyStore;

        let (_dir, store) = temp_store().await;
        insert_receipts(&store, vec![(1, receipt_at(Epoch(5))), (2, receipt_at(Epoch(6)))]).await;
        insert_receipts(&store, vec![(3, receipt_at(Epoch(20)))]).await;

        store
            .with_write_tx(move |tx| tx.prune_transaction_receipts_before_epoch(Epoch(10), 100))
            .await
            .unwrap();

        let econ = ReadOnlyStore::new(store.clone()).get_tari_economics().await.unwrap();
        assert_eq!(econ.transaction_receipt_count, 3);
    }

    #[tokio::test]
    async fn prune_events_removes_only_those_committed_before_the_cutoff() {
        let (_dir, store) = temp_store().await;
        insert_receipts(&store, vec![
            (1, receipt_with_events_at(Epoch(5), 2)),
            (2, receipt_with_events_at(Epoch(10), 1)),
        ])
        .await;
        assert_eq!(event_epochs(&store).await, vec![5, 5, 10]);

        let num_pruned = store
            .with_write_tx(move |tx| tx.prune_events_before_epoch(Epoch(10), 100))
            .await
            .unwrap();
        assert_eq!(num_pruned, 2);
        assert_eq!(event_epochs(&store).await, vec![10]);

        // Events and receipts age out independently.
        store
            .with_read_tx(move |tx| tx.get_transaction_receipt(&receipt_address(1)))
            .await
            .unwrap();
    }

    /// As with transactions, the prune selects run under the write lock on every pass, so they must be
    /// served from the epoch indexes rather than a table scan.
    #[tokio::test]
    async fn receipt_and_event_prune_selects_are_served_by_the_epoch_indexes() {
        let (_dir, store) = temp_store().await;

        let plan = query_plan(
            &store,
            "select id from transaction_receipts where epoch < 100 order by epoch asc limit 500",
        )
        .await;
        assert!(plan.contains("transaction_receipts_epoch_idx"), "{plan}");
        assert!(!plan.contains("SCAN transaction_receipts"), "{plan}");

        let plan = query_plan(
            &store,
            "select id from events where epoch < 100 order by epoch asc limit 500",
        )
        .await;
        assert!(plan.contains("events_epoch_idx"), "{plan}");
        assert!(!plan.contains("SCAN events"), "{plan}");
    }

    /// Indexers upgrading in place carry receipts and events stored before either recorded an epoch.
    /// The migration must date them from the receipt, or enabling retention would prune the whole
    /// backlog as epoch 0 on its first pass.
    #[tokio::test]
    async fn the_retention_migration_dates_existing_receipts_and_events() {
        use diesel_migrations::MigrationHarness;

        use crate::{
            storage_sqlite::serialization::{serialize_hex, serialize_json},
            store::ReadOnlyStore,
        };

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("indexer.db");
        {
            let mut conn = SqliteConnection::establish(path.to_str().unwrap()).unwrap();
            // Only the initial migration: the schema an indexer upgrading in place already has.
            conn.run_next_migration(MIGRATIONS).unwrap();

            let dated = serialize_hex(receipt_address(1).as_object_key());
            let orphan = serialize_hex(receipt_address(2).as_object_key());
            sql_query("insert into transaction_receipts (address, data) values (?, ?)")
                .bind::<diesel::sql_types::Text, _>(&dated)
                .bind::<diesel::sql_types::Text, _>(serialize_json(&receipt_at(Epoch(7))).unwrap())
                .execute(&mut conn)
                .unwrap();
            for tx_hash in [&dated, &orphan] {
                sql_query(
                    "insert into events (template_address, tx_hash, topic, payload) values ('00', ?, 'test', '{}')",
                )
                .bind::<diesel::sql_types::Text, _>(tx_hash)
                .execute(&mut conn)
                .unwrap();
            }
        }

        let store = SqliteIndexerStore::try_create(path).unwrap();

        let receipt_epochs = store
            .with_read_tx(|tx| {
                sql_query("select epoch from transaction_receipts")
                    .load::<EpochRow>(tx.connection())
                    .map_err(|e| StorageError::general("receipt_epochs", e))
            })
            .await
            .unwrap();
        assert_eq!(receipt_epochs.iter().map(|r| r.epoch).collect::<Vec<_>>(), vec![7]);
        // An event whose receipt this indexer never stored has nothing to date it by.
        assert_eq!(event_epochs(&store).await, vec![7, 0]);

        let econ = ReadOnlyStore::new(store.clone()).get_tari_economics().await.unwrap();
        assert_eq!(econ.transaction_receipt_count, 1);
    }

    #[cfg(feature = "metrics")]
    #[tokio::test]
    async fn file_stats_report_the_database_and_its_wal() {
        let (_dir, store) = temp_store().await;
        insert_receipts(&store, vec![(1, receipt_with_events_at(Epoch(1), 3))]).await;

        let stats = store.file_stats().await.unwrap();
        assert!(stats.db_bytes > 0);
        assert!(stats.wal_bytes > 0);
        // No read transaction is open, so the checkpoint copies every frame back.
        assert_eq!(stats.wal_uncheckpointed_frames, 0);
    }

    #[cfg(feature = "metrics")]
    #[tokio::test]
    async fn a_writer_that_cannot_take_the_lock_is_recognised_as_busy() {
        let (dir, _store) = temp_store().await;
        let url = dir.path().join("indexer.db");
        let url = url.to_str().unwrap();

        let mut holder = SqliteConnection::establish(url).unwrap();
        let _held = SqliteTransaction::begin_immediate(&mut holder).unwrap();

        // A fresh connection has no busy timeout, so it gives up at once instead of after five seconds.
        let mut waiter = SqliteConnection::establish(url).unwrap();
        let err = SqliteTransaction::begin_immediate(&mut waiter)
            .err()
            .expect("lock is held");
        assert!(is_busy(&err), "{err}");
    }

    #[tokio::test]
    async fn queued_writers_leave_pooled_connections_for_readers() {
        let (_dir, store) = temp_store().await;

        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let holder = tokio::spawn({
            let store = store.clone();
            async move {
                store
                    .with_write_tx(move |_| {
                        entered_tx.send(()).unwrap();
                        std::thread::sleep(Duration::from_millis(500));
                        Ok::<_, StorageError>(())
                    })
                    .await
            }
        });
        entered_rx.await.unwrap();

        // More writers than the pool has connections, all waiting on the holder.
        let writers = (0..POOL_MAX_SIZE * 2)
            .map(|_| {
                let store = store.clone();
                tokio::spawn(async move { store.with_write_tx(|_| Ok::<_, StorageError>(())).await })
            })
            .collect::<Vec<_>>();
        tokio::time::sleep(Duration::from_millis(50)).await;

        let started = std::time::Instant::now();
        store
            .with_read_tx(|tx| tx.get_transaction_rejection_status(TransactionId::default()))
            .await
            .unwrap();
        let read_wait = started.elapsed();

        holder.await.unwrap().unwrap();
        for writer in writers {
            writer.await.unwrap().unwrap();
        }
        assert!(
            read_wait < Duration::from_millis(250),
            "reader waited {read_wait:?} for a connection"
        );
    }

    // -------------------------------- Substate Cache -------------------------------- //

    /// Matches the value bootstrap passes; the tests only need it to be well above their own offsets.
    const HEAD_TTL: Duration = Duration::from_secs(900);

    fn substate(n: u8) -> SubstateId {
        format!("component_{:064x}", n).parse().unwrap()
    }

    fn now_secs() -> u64 {
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
    }

    async fn put_entry(
        store: &SqliteIndexerStore,
        id: &SubstateId,
        version: SubstateVersion,
        verified: bool,
        cached_at: u64,
        watermark: u64,
    ) -> bool {
        let result = SubstateResult::Down { version };
        let id = id.clone();
        store
            .with_write_tx(move |tx| {
                tx.substate_cache_put(
                    &id,
                    SubstateCacheEntryRef {
                        version: Some(version),
                        substate_result: &result,
                        cached_at,
                        verified,
                        proof: None,
                    },
                    FetchWatermark::new(watermark),
                    HEAD_TTL,
                )
            })
            .await
            .unwrap()
    }

    async fn put(store: &SqliteIndexerStore, id: &SubstateId, version: SubstateVersion, watermark: u64) -> bool {
        put_entry(store, id, version, true, now_secs(), watermark).await
    }

    /// A committee member answering that `version` is live, as against the `Down` every other put
    /// helper here records.
    async fn put_up(store: &SqliteIndexerStore, id: &SubstateId, version: SubstateVersion, watermark: u64) -> bool {
        use tari_engine_types::{
            non_fungible::NonFungibleContainer,
            substate::{Substate, SubstateValue},
        };

        let result = SubstateResult::Up {
            substate: Box::new(Substate::new(
                version,
                SubstateValue::NonFungible(NonFungibleContainer::no_data()),
            )),
        };
        let id = id.clone();
        store
            .with_write_tx(move |tx| {
                tx.substate_cache_put(
                    &id,
                    SubstateCacheEntryRef {
                        version: Some(version),
                        substate_result: &result,
                        cached_at: now_secs(),
                        verified: true,
                        proof: None,
                    },
                    FetchWatermark::new(watermark),
                    HEAD_TTL,
                )
            })
            .await
            .unwrap()
    }

    /// The cached head version. `None` covers both no row at all and a row recording that the
    /// substate does not exist; use [`read_entry`] where the two must be told apart.
    async fn read(store: &SqliteIndexerStore, id: &SubstateId) -> Option<SubstateVersion> {
        read_entry(store, id).await.and_then(|entry| entry.version)
    }

    async fn read_entry(store: &SqliteIndexerStore, id: &SubstateId) -> Option<SubstateCacheEntry> {
        let id = id.clone();
        store.with_read_tx(move |tx| tx.substate_cache_get(&id)).await.unwrap()
    }

    /// Records that `id` does not exist, as an unversioned entry.
    async fn put_nonexistent(store: &SqliteIndexerStore, id: &SubstateId, watermark: u64) -> bool {
        let id = id.clone();
        store
            .with_write_tx(move |tx| {
                tx.substate_cache_put(
                    &id,
                    SubstateCacheEntryRef {
                        version: None,
                        substate_result: &SubstateResult::DoesNotExist,
                        cached_at: now_secs(),
                        verified: false,
                        proof: None,
                    },
                    FetchWatermark::new(watermark),
                    HEAD_TTL,
                )
            })
            .await
            .unwrap()
    }

    fn is_nonexistent(entry: &SubstateCacheEntry) -> bool {
        entry.version.is_none() && matches!(entry.substate_result, SubstateResult::DoesNotExist)
    }

    async fn invalidate(store: &SqliteIndexerStore, invalidation: SubstateCacheInvalidation, at: u64) {
        store
            .with_write_tx(move |tx| tx.substate_cache_invalidate([invalidation], StateVersion::new(at)))
            .await
            .unwrap();
    }

    /// The hole the substate version closes: the fetch is not overtaken by the transition - it
    /// started afterwards - but the member it asked is behind and answers with a version the stream
    /// has already shown this substate past. The transition deleted the row that would have ranked
    /// it, so the journal is the only thing left to refuse it.
    #[tokio::test]
    async fn a_version_the_stream_has_already_seen_past_is_refused() {
        let (_d, store) = temp_store().await;
        let id = substate(1);
        assert!(put(&store, &id, SubstateVersion::new(6), 100).await);

        invalidate(
            &store,
            SubstateCacheInvalidation::created(&id, SubstateVersion::new(7)).unwrap(),
            105,
        )
        .await;
        assert!(read(&store, &id).await.is_none());

        // A fetch that began after the creation committed, answered by a member still on v6.
        assert!(!put(&store, &id, SubstateVersion::new(6), 110).await);
        assert!(read(&store, &id).await.is_none());
    }

    /// The version the stream showed is a floor, not a target: the substate can move on, and a fetch
    /// that catches up with it must still be recorded.
    #[tokio::test]
    async fn a_version_at_or_above_the_one_the_stream_saw_is_recorded() {
        let (_d, store) = temp_store().await;
        let id = substate(1);

        invalidate(
            &store,
            SubstateCacheInvalidation::created(&id, SubstateVersion::new(7)).unwrap(),
            105,
        )
        .await;
        assert!(put(&store, &id, SubstateVersion::new(7), 110).await);
        assert_eq!(read(&store, &id).await, Some(SubstateVersion::new(7)));
        assert!(put(&store, &id, SubstateVersion::new(9), 110).await);
        assert_eq!(read(&store, &id).await, Some(SubstateVersion::new(9)));
    }

    /// A destroy shows the version it names as reached just as a creation does, so a member still
    /// answering below it is refused the same way.
    #[tokio::test]
    async fn a_destroy_records_the_version_it_saw() {
        let (_d, store) = temp_store().await;
        let id = substate(1);

        invalidate(
            &store,
            SubstateCacheInvalidation::destroyed(id.clone(), SubstateVersion::new(7)),
            105,
        )
        .await;
        assert!(!put(&store, &id, SubstateVersion::new(6), 110).await);
        // The destroyed version is itself a legitimate head: it is down, which is what a lookup for
        // it answers.
        assert!(put(&store, &id, SubstateVersion::new(7), 110).await);
    }

    /// A destroy with no successor leaves the version it named as the floor, and that version is
    /// spent. A member still holding it live offers an `Up` at exactly the floor, which the version
    /// alone does not refuse.
    #[tokio::test]
    async fn a_destroy_with_no_successor_refuses_a_live_head_at_the_version_it_spent() {
        let (_d, store) = temp_store().await;
        let id = substate(1);

        invalidate(
            &store,
            SubstateCacheInvalidation::destroyed(id.clone(), SubstateVersion::new(7)),
            105,
        )
        .await;
        assert!(!put_up(&store, &id, SubstateVersion::new(7), 110).await);
        assert!(read(&store, &id).await.is_none());
    }

    /// The other half of the same floor: a lookup for a destroyed version answers `Down`, so that
    /// result is the substate's legitimate head and is recorded.
    #[tokio::test]
    async fn a_destroy_with_no_successor_still_admits_the_down_at_that_version() {
        let (_d, store) = temp_store().await;
        let id = substate(1);

        invalidate(
            &store,
            SubstateCacheInvalidation::destroyed(id.clone(), SubstateVersion::new(7)),
            105,
        )
        .await;
        assert!(put(&store, &id, SubstateVersion::new(7), 110).await);
        assert_eq!(read(&store, &id).await, Some(SubstateVersion::new(7)));
    }

    /// A destroy with a successor is not a spend of the floor: the creation raises it to the version
    /// that is live, which must be admitted as an `Up`. The two reach the journal in no stated order.
    #[tokio::test]
    async fn a_destroy_with_a_successor_admits_the_created_version_live() {
        for reversed in [false, true] {
            let (_d, store) = temp_store().await;
            let id = substate(1);
            let mut batch = vec![
                SubstateCacheInvalidation::destroyed(id.clone(), SubstateVersion::new(6)),
                SubstateCacheInvalidation::created(&id, SubstateVersion::new(7)).unwrap(),
            ];
            if reversed {
                batch.reverse();
            }
            store
                .with_write_tx(move |tx| tx.substate_cache_invalidate(batch, StateVersion::new(105)))
                .await
                .unwrap();

            assert!(!put_up(&store, &id, SubstateVersion::new(6), 110).await);
            assert!(put_up(&store, &id, SubstateVersion::new(7), 110).await);
            assert_eq!(read(&store, &id).await, Some(SubstateVersion::new(7)));
        }
    }

    /// A substate created and destroyed within one batch reaches the journal at a single version
    /// from both sides. The spend is the later of the two whichever order they arrive in, so it
    /// stands.
    #[tokio::test]
    async fn a_version_both_created_and_destroyed_in_one_batch_is_spent() {
        for reversed in [false, true] {
            let (_d, store) = temp_store().await;
            let id = substate(1);
            let mut batch = vec![
                SubstateCacheInvalidation::created(&id, SubstateVersion::new(7)).unwrap(),
                SubstateCacheInvalidation::destroyed(id.clone(), SubstateVersion::new(7)),
            ];
            if reversed {
                batch.reverse();
            }
            store
                .with_write_tx(move |tx| tx.substate_cache_invalidate(batch, StateVersion::new(105)))
                .await
                .unwrap();

            assert!(!put_up(&store, &id, SubstateVersion::new(7), 110).await);
            assert!(put(&store, &id, SubstateVersion::new(7), 110).await);
        }
    }

    /// One transaction downs a version and ups the next, and the two reach the journal in no stated
    /// order. The floor is the highest version either showed, whichever was journalled last.
    #[tokio::test]
    async fn the_floor_is_the_highest_version_a_batch_showed() {
        let (_d, store) = temp_store().await;
        let id = substate(1);

        let batch = [
            SubstateCacheInvalidation::created(&id, SubstateVersion::new(7)).unwrap(),
            SubstateCacheInvalidation::destroyed(id.clone(), SubstateVersion::new(6)),
        ];
        store
            .with_write_tx(move |tx| tx.substate_cache_invalidate(batch, StateVersion::new(105)))
            .await
            .unwrap();

        assert!(!put(&store, &id, SubstateVersion::new(6), 110).await);
        assert!(put(&store, &id, SubstateVersion::new(7), 110).await);
    }

    /// Nonexistence is settled by f + 1 members rather than one, so a single member being behind
    /// cannot produce it, and the one that follows a destroy is legitimate.
    #[tokio::test]
    async fn a_nonexistence_after_a_destroy_is_still_recorded() {
        let (_d, store) = temp_store().await;
        let id = substate(1);

        invalidate(
            &store,
            SubstateCacheInvalidation::destroyed(id.clone(), SubstateVersion::new(7)),
            105,
        )
        .await;
        assert!(put_nonexistent(&store, &id, 110).await);
        assert!(is_nonexistent(&read_entry(&store, &id).await.unwrap()));
    }

    /// The point of journalling a first creation: it is the only transition that can retract the
    /// claim that a substate does not exist.
    #[tokio::test]
    async fn a_first_creation_retires_the_nonexistence_it_denies() {
        let (_d, store) = temp_store().await;
        let id = substate(1);
        assert!(put_nonexistent(&store, &id, 100).await);
        assert!(is_nonexistent(&read_entry(&store, &id).await.unwrap()));

        invalidate(
            &store,
            SubstateCacheInvalidation::created(&id, SubstateVersion::ZERO).unwrap(),
            105,
        )
        .await;
        assert!(read_entry(&store, &id).await.is_none());
    }

    /// A creation retires the nonexistence whatever version it lands at, not only the first.
    #[tokio::test]
    async fn a_later_creation_also_retires_the_nonexistence() {
        let (_d, store) = temp_store().await;
        let id = substate(1);
        assert!(put_nonexistent(&store, &id, 100).await);

        invalidate(
            &store,
            SubstateCacheInvalidation::created(&id, SubstateVersion::new(6)).unwrap(),
            105,
        )
        .await;
        assert!(read_entry(&store, &id).await.is_none());
    }

    /// `DoesNotExist` says the substate has no live version, which a destroy makes more true.
    #[tokio::test]
    async fn a_destroy_leaves_a_cached_nonexistence_alone() {
        let (_d, store) = temp_store().await;
        let id = substate(1);
        assert!(put_nonexistent(&store, &id, 100).await);

        invalidate(
            &store,
            SubstateCacheInvalidation::destroyed(id.clone(), SubstateVersion::new(6)),
            105,
        )
        .await;
        assert!(is_nonexistent(&read_entry(&store, &id).await.unwrap()));
    }

    /// The race the journal exists to close: the substate is created while the committee fetch that
    /// answered `DoesNotExist` is still in flight, so the delete runs before there is a row to
    /// delete and only the journal can stop the write.
    #[tokio::test]
    async fn a_creation_landing_mid_fetch_vetoes_the_nonexistence() {
        let (_d, store) = temp_store().await;
        let id = substate(1);

        // The fetch captured the watermark at 100; the creation commits at 105 while it is in flight.
        invalidate(
            &store,
            SubstateCacheInvalidation::created(&id, SubstateVersion::ZERO).unwrap(),
            105,
        )
        .await;
        assert!(!put_nonexistent(&store, &id, 100).await);
        assert!(read_entry(&store, &id).await.is_none());
    }

    /// Nothing would ever retract a nonexistence recorded for a substate no first creation journals,
    /// so the write is refused rather than left to age out.
    #[tokio::test]
    async fn a_nonexistence_is_refused_where_no_transition_would_retract_it() {
        let (_d, store) = temp_store().await;
        let receipt: SubstateId = format!("txreceipt_{:064x}", 1).parse().unwrap();
        assert!(!put_nonexistent(&store, &receipt, 100).await);
        assert!(read_entry(&store, &receipt).await.is_none());
    }

    /// Nonexistence ranks below every version: a real head displaces it, and it never walks one back.
    #[tokio::test]
    async fn a_nonexistence_yields_to_any_head() {
        let (_d, store) = temp_store().await;
        let id = substate(1);
        assert!(put_nonexistent(&store, &id, 100).await);
        assert!(put(&store, &id, SubstateVersion::ZERO, 100).await);
        assert_eq!(read(&store, &id).await, Some(SubstateVersion::ZERO));

        // ...and cannot displace one that is verified and current.
        assert!(!put_nonexistent(&store, &id, 100).await);
        assert_eq!(read(&store, &id).await, Some(SubstateVersion::ZERO));
    }

    #[tokio::test]
    async fn a_cached_head_is_held_until_a_transition_retires_it() {
        let (_d, store) = temp_store().await;
        let id = substate(1);
        assert!(put(&store, &id, SubstateVersion::new(5), 100).await);
        assert_eq!(read(&store, &id).await, Some(SubstateVersion::new(5)));

        invalidate(
            &store,
            SubstateCacheInvalidation::created(&id, SubstateVersion::new(6)).unwrap(),
            105,
        )
        .await;
        assert_eq!(read(&store, &id).await, None);
    }

    #[tokio::test]
    async fn a_destroy_retires_the_version_it_names() {
        let (_d, store) = temp_store().await;
        let id = substate(1);
        assert!(put(&store, &id, SubstateVersion::new(5), 100).await);

        invalidate(
            &store,
            SubstateCacheInvalidation::destroyed(id.clone(), SubstateVersion::new(5)),
            105,
        )
        .await;
        assert_eq!(read(&store, &id).await, None);
    }

    /// A head can legitimately run ahead of the transition stream, having come straight from the
    /// committee. Retiring it on a transition it already accounts for would cost a round trip on every
    /// read of a substate whose shard is catching up.
    #[tokio::test]
    async fn a_transition_leaves_a_higher_cached_head_alone() {
        let (_d, store) = temp_store().await;
        let id = substate(1);
        assert!(put(&store, &id, SubstateVersion::new(9), 100).await);

        invalidate(
            &store,
            SubstateCacheInvalidation::created(&id, SubstateVersion::new(7)).unwrap(),
            105,
        )
        .await;
        invalidate(
            &store,
            SubstateCacheInvalidation::destroyed(id.clone(), SubstateVersion::new(8)),
            106,
        )
        .await;
        assert_eq!(read(&store, &id).await, Some(SubstateVersion::new(9)));
    }

    /// A committee member that is behind answers with a version below the head already held. Taking it
    /// would walk the cached head backwards and reopen the window this cache exists to close.
    #[tokio::test]
    async fn a_lower_version_does_not_displace_the_cached_head() {
        let (_d, store) = temp_store().await;
        let id = substate(1);
        assert!(put(&store, &id, SubstateVersion::new(6), 100).await);
        assert!(!put(&store, &id, SubstateVersion::new(5), 100).await);
        assert_eq!(read(&store, &id).await, Some(SubstateVersion::new(6)));
    }

    /// The batch RPC carries no proofs, so a single validator can park an unverified head above any
    /// version the substate reached. No transition retires a version above the head, so without this
    /// the proven head could never be written and the entry would be dead until eviction.
    #[tokio::test]
    async fn a_verified_result_displaces_an_unverified_head() {
        let (_d, store) = temp_store().await;
        let id = substate(1);
        assert!(put_entry(&store, &id, SubstateVersion::new(999), false, now_secs(), 100).await);
        assert!(put_entry(&store, &id, SubstateVersion::new(6), true, now_secs(), 100).await);
        assert_eq!(read(&store, &id).await, Some(SubstateVersion::new(6)));
    }

    /// A committee member that is behind can prove an older version against an older signed root, which
    /// the trusted-root ring accepts by design. A proof attests only that the version existed, so a
    /// proven head is a lower bound that no amount of elapsed time may walk back.
    #[tokio::test]
    async fn an_aged_verified_head_is_still_not_walked_backwards() {
        let (_d, store) = temp_store().await;
        let id = substate(1);
        let aged = now_secs() - HEAD_TTL.as_secs() - 1;
        assert!(put_entry(&store, &id, SubstateVersion::new(10), true, aged, 100).await);
        assert!(!put_entry(&store, &id, SubstateVersion::new(6), true, now_secs(), 100).await);
        assert_eq!(read(&store, &id).await, Some(SubstateVersion::new(10)));
    }

    /// An unverified head is not a lower bound on anything, and with proof verification off nothing
    /// outranks it, so ageing it out is the only way a wrong one is ever corrected.
    #[tokio::test]
    async fn an_aged_unverified_head_does_not_block_a_lower_version() {
        let (_d, store) = temp_store().await;
        let id = substate(1);
        let stale = now_secs() - HEAD_TTL.as_secs() - 1;
        assert!(put_entry(&store, &id, SubstateVersion::new(999), false, stale, 100).await);
        assert!(put_entry(&store, &id, SubstateVersion::new(6), false, now_secs(), 100).await);
        assert_eq!(read(&store, &id).await, Some(SubstateVersion::new(6)));
    }

    #[tokio::test]
    async fn a_write_is_vetoed_by_a_transition_that_landed_during_the_fetch() {
        let (_d, store) = temp_store().await;
        let id = substate(1);
        invalidate(
            &store,
            SubstateCacheInvalidation::created(&id, SubstateVersion::new(6)).unwrap(),
            105,
        )
        .await;

        assert!(!put(&store, &id, SubstateVersion::new(6), 100).await);
        assert_eq!(read(&store, &id).await, None);

        // The same result fetched against a watermark that already covers the transition is current.
        assert!(put(&store, &id, SubstateVersion::new(6), 105).await);
        assert_eq!(read(&store, &id).await, Some(SubstateVersion::new(6)));
    }

    /// Retirements driven by a finalized result are counted like the stream's own, so the counter
    /// means what its name says.
    #[tokio::test]
    async fn retiring_ahead_of_the_stream_reports_what_it_retired() {
        let (_d, store) = temp_store().await;
        let held = substate(1);
        let spent = substate(2);
        let untouched = substate(3);
        assert!(put(&store, &held, SubstateVersion::new(6), 100).await);
        assert!(put(&store, &spent, SubstateVersion::new(6), 100).await);
        assert!(put(&store, &untouched, SubstateVersion::new(6), 100).await);

        let ahead = StateVersion::new(101);
        let invalidations = vec![
            // Below the cached head, so it retires nothing.
            (
                SubstateCacheInvalidation::created(&held, SubstateVersion::new(4)).unwrap(),
                ahead,
            ),
            (
                SubstateCacheInvalidation::destroyed(spent.clone(), SubstateVersion::new(6)),
                ahead,
            ),
        ];
        let retired = store
            .with_write_tx(move |tx| tx.substate_cache_retire_ahead(invalidations))
            .await
            .unwrap();

        assert_eq!(retired, 1);
        assert_eq!(read(&store, &held).await, Some(SubstateVersion::new(6)));
        assert_eq!(read(&store, &spent).await, None);
        assert_eq!(read(&store, &untouched).await, Some(SubstateVersion::new(6)));
    }

    #[tokio::test]
    async fn pruning_evicts_down_to_the_cap_and_expires_the_journal() {
        let (_d, store) = temp_store().await;
        // Descending `cached_at`, so the substates with the lowest n are the oldest and evicted first.
        let now = now_secs();
        for n in 0..5u8 {
            assert!(
                put_entry(
                    &store,
                    &substate(n),
                    SubstateVersion::new(1),
                    true,
                    now - u64::from(4 - n),
                    100
                )
                .await
            );
        }
        invalidate(
            &store,
            SubstateCacheInvalidation::created(&substate(9), SubstateVersion::new(1)).unwrap(),
            105,
        )
        .await;

        store
            .with_write_tx(|tx| tx.substate_cache_prune(Duration::ZERO, 2))
            .await
            .unwrap();

        let mut remaining = Vec::new();
        for n in 0..5u8 {
            if read(&store, &substate(n)).await.is_some() {
                remaining.push(n);
            }
        }
        assert_eq!(remaining, vec![3, 4], "eviction did not take the oldest entries");

        // With the journal expired, a fetch that started before the transition is no longer vetoed.
        assert!(put(&store, &substate(9), SubstateVersion::new(1), 100).await);
    }

    async fn store_with_events(topics: &[&str]) -> (tempfile::TempDir, SqliteIndexerStore) {
        let (dir, store) = temp_store().await;
        insert_test_events(&dir.path().join("indexer.db"), topics);
        (dir, store)
    }

    fn topic_query(topic: &str, wildcard_scan_limit: u32) -> crate::store::EventQuery {
        crate::store::EventQuery {
            topic: Some(topic.to_string()),
            substate_id: None,
            template_address: None,
            resource_address: None,
            wildcard_scan_limit,
        }
    }

    fn page_ids(page: &crate::store::EventsPage) -> Vec<i64> {
        page.events.iter().map(|(id, _, _)| *id).collect()
    }

    #[tokio::test]
    async fn a_wildcard_topic_filter_matches_underscores_literally() {
        let (_dir, store) = store_with_events(&["my_template.minted", "myXtemplate.minted"]).await;
        let query = topic_query("my_template.*", 100);

        let q = query.clone();
        let page = store
            .with_read_tx(move |tx| tx.get_events(&q, None, 0, 10))
            .await
            .unwrap();
        assert_eq!(page_ids(&page), vec![1]);

        let page = store
            .with_read_tx(move |tx| tx.get_events_after_id(&query, 0, 10))
            .await
            .unwrap();
        assert_eq!(page_ids(&page), vec![1]);
    }

    #[tokio::test]
    async fn a_wildcard_matches_exactly_one_topic_segment() {
        let (_dir, store) = store_with_events(&["std.vault.deposit", "std.mint", "std.vault.deposit.extra"]).await;

        for (pattern, expected) in [("std.*", vec![2]), ("std.*.deposit", vec![1])] {
            let query = topic_query(pattern, 100);
            let page = store
                .with_read_tx(move |tx| tx.get_events_after_id(&query, 0, 10))
                .await
                .unwrap();
            assert_eq!(page_ids(&page), expected, "pattern {pattern}");
        }
    }

    #[tokio::test]
    async fn a_wildcard_query_examines_at_most_the_scan_limit_and_resumes_by_cursor() {
        // Only the oldest of 25 events matches.
        let mut topics = vec!["rare.hit"];
        topics.extend(std::iter::repeat_n("common.miss", 24));
        let (_dir, store) = store_with_events(&topics).await;
        let query = topic_query("rare.*", 10);

        let mut cursor = None;
        let mut calls = 0;
        let mut found = Vec::new();
        loop {
            let q = query.clone();
            let page = store
                .with_read_tx(move |tx| tx.get_events(&q, cursor, 0, 5))
                .await
                .unwrap();
            calls += 1;
            found.extend(page_ids(&page));
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
        assert_eq!(found, vec![1]);
        // 25 rows at 10 per call: the first two calls return empty pages that still carry a cursor.
        assert_eq!(calls, 3);

        let mut after = 0;
        let mut found = Vec::new();
        loop {
            let q = topic_query("common.*", 10);
            let page = store
                .with_read_tx(move |tx| tx.get_events_after_id(&q, after, 100))
                .await
                .unwrap();
            found.extend(page_ids(&page));
            match page.next_cursor {
                Some(next) => after = next,
                None => break,
            }
        }
        assert_eq!(found, (2..=25).collect::<Vec<_>>());
    }

    #[tokio::test]
    async fn before_id_pages_through_every_event_newest_first() {
        let (_dir, store) = store_with_events(&["a.b"; 7]).await;
        let query = topic_query("a.b", 100);

        let mut cursor = None;
        let mut found = Vec::new();
        loop {
            let q = query.clone();
            let page = store
                .with_read_tx(move |tx| tx.get_events(&q, cursor, 0, 3))
                .await
                .unwrap();
            found.extend(page_ids(&page));
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
        assert_eq!(found, vec![7, 6, 5, 4, 3, 2, 1]);
    }

    async fn put_with_proof(
        store: &SqliteIndexerStore,
        id: &SubstateId,
        version: SubstateVersion,
        cached_at: u64,
        marker: u8,
    ) -> bool {
        let result = SubstateResult::Down { version };
        let proof = tari_validator_node_rpc::client::SubstateProofData {
            substate_value_proof: vec![marker],
            commit_proof: vec![marker; 2],
            proof_epoch: u64::from(marker),
            substate_down_proof: None,
            destroyed_at_state_version: None,
        };
        let id = id.clone();
        store
            .with_write_tx(move |tx| {
                tx.substate_cache_put(
                    &id,
                    SubstateCacheEntryRef {
                        version: Some(version),
                        substate_result: &result,
                        cached_at,
                        verified: true,
                        proof: Some(&proof),
                    },
                    FetchWatermark::new(10),
                    HEAD_TTL,
                )
            })
            .await
            .unwrap()
    }

    /// The value-proof marker of the proof held for `version`, if one is.
    async fn read_proof_marker(store: &SqliteIndexerStore, id: &SubstateId, version: SubstateVersion) -> Option<u8> {
        let id = id.clone();
        let proof = store
            .with_read_tx(move |tx| tx.substate_cache_proof_get(&id, version))
            .await
            .unwrap()?;
        assert_eq!(proof.commit_proof, vec![proof.substate_value_proof[0]; 2]);
        assert_eq!(proof.proof_epoch, u64::from(proof.substate_value_proof[0]));
        Some(proof.substate_value_proof[0])
    }

    async fn prune(store: &SqliteIndexerStore, max_entries: usize) -> usize {
        store
            .with_write_tx(move |tx| tx.substate_cache_prune(Duration::from_secs(300), max_entries))
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn a_proof_outlives_pruning_only_while_its_version_is_the_cached_head() {
        let (_dir, store) = temp_store().await;
        let id = substate(1);

        assert!(put_with_proof(&store, &id, SubstateVersion::new(1), now_secs(), 1).await);
        prune(&store, 100).await;
        assert_eq!(read_proof_marker(&store, &id, SubstateVersion::new(1)).await, Some(1));

        assert!(put_with_proof(&store, &id, SubstateVersion::new(2), now_secs(), 2).await);
        prune(&store, 100).await;
        assert_eq!(read_proof_marker(&store, &id, SubstateVersion::new(1)).await, None);
        assert_eq!(read_proof_marker(&store, &id, SubstateVersion::new(2)).await, Some(2));
    }

    #[tokio::test]
    async fn a_refused_write_records_no_proof() {
        let (_dir, store) = temp_store().await;
        let id = substate(1);

        assert!(put_with_proof(&store, &id, SubstateVersion::new(2), now_secs(), 2).await);
        assert!(!put_with_proof(&store, &id, SubstateVersion::new(1), now_secs(), 1).await);

        assert_eq!(read_proof_marker(&store, &id, SubstateVersion::new(1)).await, None);
    }

    #[tokio::test]
    async fn an_evicted_entry_takes_its_proof_with_it() {
        let (_dir, store) = temp_store().await;
        let (older, newer) = (substate(1), substate(2));

        assert!(put_with_proof(&store, &older, SubstateVersion::new(1), now_secs() - 10, 1).await);
        assert!(put_with_proof(&store, &newer, SubstateVersion::new(1), now_secs(), 2).await);
        assert_eq!(prune(&store, 1).await, 1);

        assert_eq!(read_proof_marker(&store, &older, SubstateVersion::new(1)).await, None);
        assert_eq!(
            read_proof_marker(&store, &newer, SubstateVersion::new(1)).await,
            Some(2)
        );
    }
}
