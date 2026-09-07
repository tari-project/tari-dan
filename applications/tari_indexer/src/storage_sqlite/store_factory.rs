//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{fmt::Debug, fs::create_dir_all, path::PathBuf, time::Duration};

use async_trait::async_trait;
use deadpool_diesel::{
    Runtime,
    sqlite::{Hook, HookError, Manager, Pool},
};
use diesel::{Connection, RunQueryDsl, SqliteConnection, sql_query};
use diesel_migrations::{EmbeddedMigrations, MigrationHarness};
use tari_ootle_storage::StorageError;
use tari_ootle_storage_sqlite::{SqliteTransaction, error::SqliteStorageError};

use crate::{
    storage_sqlite::{reader::SqliteStoreReadTransaction, writer::SqliteStoreWriteTransaction},
    store::{IndexerStore, IndexerStoreReader, IndexerStoreWriteTransaction},
};

const LOG_TARGET: &str = "tari::indexer::storage_sqlite";
const POOL_MAX_SIZE: usize = 16;
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub struct SqliteIndexerStore {
    pool: Pool,
}

impl SqliteIndexerStore {
    pub fn try_create(path: PathBuf) -> Result<Self, StorageError> {
        create_dir_all(path.parent().unwrap()).map_err(|_| StorageError::FileSystemPathDoesNotExist)?;

        let database_url = path.to_str().expect("database_url utf-8 error").to_string();

        // Run migrations on a one-shot connection before opening the pool, so pooled connections
        // never observe a partially-migrated schema.
        let mut migration_conn = SqliteConnection::establish(&database_url).map_err(SqliteStorageError::from)?;
        apply_pragmas(&mut migration_conn).map_err(|source| SqliteStorageError::DieselError {
            source,
            operation: "set pragma",
        })?;
        pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!("./src/storage_sqlite/migrations");
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

        Ok(Self { pool })
    }

    async fn acquire(&self) -> Result<deadpool_diesel::sqlite::Connection, StorageError> {
        self.pool.get().await.map_err(|e| StorageError::General {
            details: format!("Failed to acquire sqlite connection from pool: {}", e),
        })
    }
}

impl Debug for SqliteIndexerStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SqliteIndexerStore {{ pool: ... }}")
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
        let conn = self.acquire().await?;
        let result: Result<R, E> = conn
            .interact(move |c| -> Result<R, E> {
                let inner = SqliteTransaction::begin_immediate(c)
                    .map_err(StorageError::from)
                    .map_err(E::from)?;
                let mut tx = SqliteStoreWriteTransaction::new(inner);
                match f(&mut tx) {
                    Ok(r) => {
                        tx.commit().map_err(E::from)?;
                        Ok(r)
                    },
                    Err(e) => {
                        if let Err(err) = tx.rollback() {
                            log::error!(target: LOG_TARGET, "Failed to rollback transaction: {}", err);
                        }
                        Err(e)
                    },
                }
            })
            .await
            .map_err(|e| StorageError::General {
                details: format!("Pool interact panicked: {}", e),
            })?;
        result
    }
}

fn apply_pragmas(conn: &mut SqliteConnection) -> Result<(), diesel::result::Error> {
    let busy_timeout_ms = BUSY_TIMEOUT.as_millis();
    sql_query("PRAGMA journal_mode = WAL;").execute(conn)?;
    sql_query("PRAGMA synchronous = NORMAL;").execute(conn)?;
    sql_query("PRAGMA foreign_keys = ON;").execute(conn)?;
    sql_query(format!("PRAGMA busy_timeout = {};", busy_timeout_ms)).execute(conn)?;
    Ok(())
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
    use tari_ootle_common_types::{Epoch, NodeHeight, ShardGroup, StateVersion};
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
            version: 0,
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

    /// The `2026-08-25-000000_transaction_source` migration backfills `retention_epoch` from the
    /// receipt via `json_extract(data, '$.epoch')`. If the receipt did not serialise its epoch as a
    /// bare number at that path the backfill would silently resolve to NULL and fall through.
    #[tokio::test]
    async fn a_receipt_exposes_its_epoch_to_the_backfill_json_path() {
        #[derive(diesel::QueryableByName)]
        struct EpochRow {
            #[diesel(sql_type = diesel::sql_types::BigInt)]
            epoch: i64,
        }

        let (_dir, store) = temp_store().await;

        let receipt_address = TransactionId::default().into_receipt_address();
        store
            .with_write_tx(move |tx| {
                tx.batch_insert_transaction_receipts([(receipt_address, receipt_at(Epoch(77)))], &[])
            })
            .await
            .unwrap();

        let rows = store
            .with_read_tx(|tx| {
                sql_query("select json_extract(data, '$.epoch') as epoch from transaction_receipts")
                    .load::<EpochRow>(tx.connection())
                    .map_err(|e| StorageError::general("json_extract epoch", e))
            })
            .await
            .unwrap();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].epoch, 77);
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
        version: u32,
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
                    },
                    FetchWatermark::new(watermark),
                    HEAD_TTL,
                )
            })
            .await
            .unwrap()
    }

    async fn put(store: &SqliteIndexerStore, id: &SubstateId, version: u32, watermark: u64) -> bool {
        put_entry(store, id, version, true, now_secs(), watermark).await
    }

    /// The cached head version. `None` covers both no row at all and a row recording that the
    /// substate does not exist; use [`read_entry`] where the two must be told apart.
    async fn read(store: &SqliteIndexerStore, id: &SubstateId) -> Option<u32> {
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

    /// The point of journalling a first creation: it is the only transition that can retract the
    /// claim that a substate does not exist.
    #[tokio::test]
    async fn a_first_creation_retires_the_nonexistence_it_denies() {
        let (_d, store) = temp_store().await;
        let id = substate(1);
        assert!(put_nonexistent(&store, &id, 100).await);
        assert!(is_nonexistent(&read_entry(&store, &id).await.unwrap()));

        invalidate(&store, SubstateCacheInvalidation::created(&id, 0).unwrap(), 105).await;
        assert!(read_entry(&store, &id).await.is_none());
    }

    /// A creation retires the nonexistence whatever version it lands at, not only the first.
    #[tokio::test]
    async fn a_later_creation_also_retires_the_nonexistence() {
        let (_d, store) = temp_store().await;
        let id = substate(1);
        assert!(put_nonexistent(&store, &id, 100).await);

        invalidate(&store, SubstateCacheInvalidation::created(&id, 6).unwrap(), 105).await;
        assert!(read_entry(&store, &id).await.is_none());
    }

    /// `DoesNotExist` says the substate has no live version, which a destroy makes more true.
    #[tokio::test]
    async fn a_destroy_leaves_a_cached_nonexistence_alone() {
        let (_d, store) = temp_store().await;
        let id = substate(1);
        assert!(put_nonexistent(&store, &id, 100).await);

        invalidate(&store, SubstateCacheInvalidation::destroyed(id.clone(), 6), 105).await;
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
        invalidate(&store, SubstateCacheInvalidation::created(&id, 0).unwrap(), 105).await;
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
        assert!(put(&store, &id, 0, 100).await);
        assert_eq!(read(&store, &id).await, Some(0));

        // ...and cannot displace one that is verified and current.
        assert!(!put_nonexistent(&store, &id, 100).await);
        assert_eq!(read(&store, &id).await, Some(0));
    }

    #[tokio::test]
    async fn a_cached_head_is_held_until_a_transition_retires_it() {
        let (_d, store) = temp_store().await;
        let id = substate(1);
        assert!(put(&store, &id, 5, 100).await);
        assert_eq!(read(&store, &id).await, Some(5));

        invalidate(&store, SubstateCacheInvalidation::created(&id, 6).unwrap(), 105).await;
        assert_eq!(read(&store, &id).await, None);
    }

    #[tokio::test]
    async fn a_destroy_retires_the_version_it_names() {
        let (_d, store) = temp_store().await;
        let id = substate(1);
        assert!(put(&store, &id, 5, 100).await);

        invalidate(&store, SubstateCacheInvalidation::destroyed(id.clone(), 5), 105).await;
        assert_eq!(read(&store, &id).await, None);
    }

    /// A head can legitimately run ahead of the transition stream, having come straight from the
    /// committee. Retiring it on a transition it already accounts for would cost a round trip on every
    /// read of a substate whose shard is catching up.
    #[tokio::test]
    async fn a_transition_leaves_a_higher_cached_head_alone() {
        let (_d, store) = temp_store().await;
        let id = substate(1);
        assert!(put(&store, &id, 9, 100).await);

        invalidate(&store, SubstateCacheInvalidation::created(&id, 7).unwrap(), 105).await;
        invalidate(&store, SubstateCacheInvalidation::destroyed(id.clone(), 8), 106).await;
        assert_eq!(read(&store, &id).await, Some(9));
    }

    /// A committee member that is behind answers with a version below the head already held. Taking it
    /// would walk the cached head backwards and reopen the window this cache exists to close.
    #[tokio::test]
    async fn a_lower_version_does_not_displace_the_cached_head() {
        let (_d, store) = temp_store().await;
        let id = substate(1);
        assert!(put(&store, &id, 6, 100).await);
        assert!(!put(&store, &id, 5, 100).await);
        assert_eq!(read(&store, &id).await, Some(6));
    }

    /// The batch RPC carries no proofs, so a single validator can park an unverified head above any
    /// version the substate reached. No transition retires a version above the head, so without this
    /// the proven head could never be written and the entry would be dead until eviction.
    #[tokio::test]
    async fn a_verified_result_displaces_an_unverified_head() {
        let (_d, store) = temp_store().await;
        let id = substate(1);
        assert!(put_entry(&store, &id, 999, false, now_secs(), 100).await);
        assert!(put_entry(&store, &id, 6, true, now_secs(), 100).await);
        assert_eq!(read(&store, &id).await, Some(6));
    }

    /// A committee member that is behind can prove an older version against an older signed root, which
    /// the trusted-root ring accepts by design. A proof attests only that the version existed, so a
    /// proven head is a lower bound that no amount of elapsed time may walk back.
    #[tokio::test]
    async fn an_aged_verified_head_is_still_not_walked_backwards() {
        let (_d, store) = temp_store().await;
        let id = substate(1);
        let aged = now_secs() - HEAD_TTL.as_secs() - 1;
        assert!(put_entry(&store, &id, 10, true, aged, 100).await);
        assert!(!put_entry(&store, &id, 6, true, now_secs(), 100).await);
        assert_eq!(read(&store, &id).await, Some(10));
    }

    /// An unverified head is not a lower bound on anything, and with proof verification off nothing
    /// outranks it, so ageing it out is the only way a wrong one is ever corrected.
    #[tokio::test]
    async fn an_aged_unverified_head_does_not_block_a_lower_version() {
        let (_d, store) = temp_store().await;
        let id = substate(1);
        let stale = now_secs() - HEAD_TTL.as_secs() - 1;
        assert!(put_entry(&store, &id, 999, false, stale, 100).await);
        assert!(put_entry(&store, &id, 6, false, now_secs(), 100).await);
        assert_eq!(read(&store, &id).await, Some(6));
    }

    #[tokio::test]
    async fn a_write_is_vetoed_by_a_transition_that_landed_during_the_fetch() {
        let (_d, store) = temp_store().await;
        let id = substate(1);
        invalidate(&store, SubstateCacheInvalidation::created(&id, 6).unwrap(), 105).await;

        assert!(!put(&store, &id, 6, 100).await);
        assert_eq!(read(&store, &id).await, None);

        // The same result fetched against a watermark that already covers the transition is current.
        assert!(put(&store, &id, 6, 105).await);
        assert_eq!(read(&store, &id).await, Some(6));
    }

    #[tokio::test]
    async fn pruning_evicts_down_to_the_cap_and_expires_the_journal() {
        let (_d, store) = temp_store().await;
        // Descending `cached_at`, so the substates with the lowest n are the oldest and evicted first.
        let now = now_secs();
        for n in 0..5u8 {
            assert!(put_entry(&store, &substate(n), 1, true, now - u64::from(4 - n), 100).await);
        }
        invalidate(
            &store,
            SubstateCacheInvalidation::created(&substate(9), 1).unwrap(),
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
        assert!(put(&store, &substate(9), 1, 100).await);
    }
}
