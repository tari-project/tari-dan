//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    fmt,
    marker::PhantomData,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

use log::*;
use rocksdb::{
    ColumnFamilyDescriptor,
    DB,
    IteratorMode,
    SingleThreaded,
    SliceTransform,
    SnapshotWithThreadMode,
    TransactionDB,
    TransactionDBOptions,
};
use serde::{Serialize, de::DeserializeOwned};
use tari_ootle_common_types::NodeAddressable;
use tari_ootle_storage::{StateStore, StorageError};

use crate::{
    column_families::cf_names,
    dbs::read_only::ReadOnlyDb,
    error::RocksDbStorageError,
    info::ColumnFamilyInfo,
    memory_budget::RocksDbMemoryBudget,
    options::DatabaseOptions,
    read_only_ctx::ReadOnlyContext,
    reader::RocksDbStateStoreReadTransaction,
    traits::{RocksDatabase, RocksReader},
    writer::RocksDbStateStoreWriteTransaction,
};

const LOG_TARGET: &str = "tari::ootle::storage::rocksdb::state_store";

pub fn all_column_families_iter() -> impl Iterator<Item = &'static str> {
    [
        cf_names::BOOKKEEPING,
        cf_names::CHAIN_METADATA,
        cf_names::TRANSACTIONS,
        cf_names::BLOCK,
        cf_names::FOREIGN_PROPOSALS,
        cf_names::CERTIFICATES,
        cf_names::SUBSTATES,
        cf_names::DIAGNOSTICS,
        cf_names::STATE_TREE,
    ]
    .into_iter()
}

/// Builds the options every column family is opened with, together with the memory budget they
/// share.
///
/// The returned `Options` is cloned once per column family; the clone carries the same [`Cache`]
/// and [`WriteBufferManager`], which is what makes the budget shared rather than per-family. Open
/// the database from this one call — building the options twice would build two budgets.
///
/// [`Cache`]: rocksdb::Cache
/// [`WriteBufferManager`]: rocksdb::WriteBufferManager
pub(crate) fn build_default_store_opts(options: &DatabaseOptions) -> (rocksdb::Options, RocksDbMemoryBudget) {
    let budget = RocksDbMemoryBudget::new(options.memory_budget_bytes, options.memtable_budget_bytes);
    let mut opts = rocksdb::Options::default();
    // Don't error if the DB exists
    opts.set_error_if_exists(false);
    // Create the DB if it doesn't exist
    opts.create_if_missing(true);
    // Create any missing column families
    opts.create_missing_column_families(true);
    // Schedule background workers instead of using the main worker thread for long-latency operations
    opts.set_avoid_unnecessary_blocking_io(true);
    // All CFs will use a 1-byte prefix extractor
    opts.set_prefix_extractor(SliceTransform::create_fixed_prefix(1));
    // Use a small memtable prefix bloom filter to speed up prefix lookups
    opts.set_memtable_prefix_bloom_ratio(0.05);
    // Better suggested defaults: https://github.com/facebook/rocksdb/wiki/Setup-Options-and-Basic-Tuning
    opts.set_max_background_jobs(6);
    opts.set_bytes_per_sync(1_048_576);
    opts.set_compaction_pri(rocksdb::CompactionPri::MinOverlappingRatio);
    opts.set_level_compaction_dynamic_level_bytes(true);
    // Memtable memory is bounded across all column families by the write buffer manager, which
    // supersedes `db_write_buffer_size`. The per-family buffer size bounds how far past the budget
    // memtable memory can drift while triggered flushes are still in flight.
    opts.set_write_buffer_size(options.write_buffer_bytes);
    opts.set_write_buffer_manager(budget.write_buffer_manager());
    let mut bb_opts = rocksdb::BlockBasedOptions::default();
    bb_opts.set_block_size(16 * 1024);
    // Index and filter blocks are the largest thing a column family holds outside its memtables, so
    // they belong inside the shared cache where they are accounted for and evictable.
    bb_opts.set_cache_index_and_filter_blocks(true);
    bb_opts.set_pin_l0_filter_and_index_blocks_in_cache(true);
    bb_opts.set_format_version(6);
    bb_opts.set_optimize_filters_for_memory(true);
    bb_opts.set_block_cache(budget.cache());

    opts.set_block_based_table_factory(&bb_opts);
    (opts, budget)
}

pub type RocksDbReadOnlyStateStore<TAddr> = RocksDbStateStore<TAddr, ReadOnlyDb>;
pub struct RocksDbStateStore<TAddr, DB = TransactionDB> {
    db: Arc<DB>,
    options: DatabaseOptions,
    memory_budget: RocksDbMemoryBudget,
    _addr: PhantomData<TAddr>,
}

/// A standalone, consistent point-in-time read view over the state store (a RocksDB snapshot) — what
/// `StateStore::create_read_tx` returns. See CONTEXT.md (read view).
pub type ReadView<'a, TAddr> = RocksDbStateStoreReadTransaction<'a, TAddr, SnapshotWithThreadMode<'a, TransactionDB>>;

impl<TAddr> RocksDbStateStore<TAddr, TransactionDB> {
    pub fn open<P: AsRef<Path>>(path: P, options: DatabaseOptions) -> Result<Self, StorageError> {
        let (rocks_opts, memory_budget) = build_default_store_opts(&options);
        let tx_db_opts = TransactionDBOptions::default();

        let cf_names = all_column_families_iter().map(|name| ColumnFamilyDescriptor::new(name, rocks_opts.clone()));
        let db = TransactionDB::<SingleThreaded>::open_cf_descriptors(&rocks_opts, &tx_db_opts, path, cf_names)
            .map_err(|e| StorageError::ConnectionError {
                reason: e.into_string(),
            })?;
        let db = Self {
            db: Arc::new(db),
            options,
            memory_budget,
            _addr: PhantomData,
        };

        Ok(db)
    }

    /// Force compact all column families in the database.
    /// This is not typically needed but can be useful for experimentation.
    pub fn compact_all<P: AsRef<Path>>(path: P) -> Result<(), StorageError> {
        let (options, _budget) = build_default_store_opts(&DatabaseOptions::default());
        let cf_names = all_column_families_iter();
        let db = DB::open_cf(&options, path, cf_names).map_err(|e| StorageError::ConnectionError {
            reason: e.into_string(),
        })?;
        for name in all_column_families_iter() {
            let handle = db.cf_handle(name).ok_or_else(|| StorageError::ConnectionError {
                reason: format!("Column family {} not found", name),
            })?;
            db.compact_range_cf(handle, None::<Vec<u8>>, None::<Vec<u8>>);
        }
        Ok(())
    }

    /// Open a consistent point-in-time read view (a RocksDB snapshot) over the database. This is the
    /// bound-free inherent form of [`tari_ootle_storage::StateStore::create_read_tx`]; see CONTEXT.md
    /// (read view).
    pub fn read_view(&self) -> ReadView<'_, TAddr> {
        let snapshot = self.db.snapshot();
        RocksDbStateStoreReadTransaction::new(&self.db, snapshot)
    }
}

impl<TAddr> RocksDbStateStore<TAddr, ReadOnlyDb> {
    pub fn open_read_only<P: AsRef<Path>>(
        path: P,
        secondary_path: P,
    ) -> Result<RocksDbReadOnlyStateStore<TAddr>, StorageError> {
        let db_options = DatabaseOptions::default();
        let (options, memory_budget) = build_default_store_opts(&db_options);
        let cf_names = all_column_families_iter().map(|name| ColumnFamilyDescriptor::new(name, options.clone()));
        let db = DB::open_cf_descriptors_as_secondary(&options, path, secondary_path, cf_names).map_err(|e| {
            StorageError::ConnectionError {
                reason: e.into_string(),
            }
        })?;

        Ok(Self {
            db: Arc::new(ReadOnlyDb::new(db)),
            _addr: PhantomData,
            options: db_options,
            memory_budget,
        })
    }

    pub fn read_only_context(&self) -> ReadOnlyContext<'_> {
        ReadOnlyContext::new(&self.db)
    }
}

impl<TAddr, DB> RocksDbStateStore<TAddr, DB> {
    /// The block cache and memtable budget every column family of this store shares. Reports live
    /// usage as well as the configured capacity, so the ceiling can be checked against what the
    /// store is really holding.
    pub fn memory_budget(&self) -> &RocksDbMemoryBudget {
        &self.memory_budget
    }
}

impl<TAddr, DB: RocksDatabase + RocksReader> RocksDbStateStore<TAddr, DB> {
    pub fn column_family_info(&self) -> Result<Vec<ColumnFamilyInfo>, RocksDbStorageError> {
        let mut cf_info = Vec::new();
        for name in all_column_families_iter() {
            let Some(handle) = self.db.cf_handle(name) else {
                warn!(
                    target: LOG_TARGET,
                    "Column family {} not found in database",
                    name
                );
                continue;
            };

            let iter = self.db.iterator_cf(handle, IteratorMode::Start);
            let mut num_entries = 0usize;
            let mut entries_bytes = 0usize;
            for rec in iter {
                let (k, v) = rec.map_err(|e| RocksDbStorageError::RocksDbError {
                    source: e,
                    operation: "column_family_info",
                })?;
                num_entries += 1;
                entries_bytes += k.len() + v.len();
            }
            cf_info.push(ColumnFamilyInfo {
                name: name.to_string(),
                num_entries,
                total_entries_bytes: entries_bytes,
            });
        }

        Ok(cf_info)
    }
}

// Manually implement the Debug implementation because `RocksDbStateStore` does not implement the Debug trait
impl<TAddr, DB> fmt::Debug for RocksDbStateStore<TAddr, DB> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "RocksDbStateStore")
    }
}

impl<TAddr: NodeAddressable + Serialize + DeserializeOwned> StateStore for RocksDbStateStore<TAddr, TransactionDB> {
    type Addr = TAddr;
    type ReadTransaction<'a>
        = ReadView<'a, Self::Addr>
    where TAddr: 'a;
    type WriteTransaction<'a>
        = RocksDbStateStoreWriteTransaction<'a, Self::Addr>
    where TAddr: 'a;

    fn create_read_tx(&self) -> Result<Self::ReadTransaction<'_>, StorageError> {
        // A standalone read view is a consistent point-in-time snapshot: it observes one committed
        // state for its whole lifetime and is safe to use concurrently with the single writer and
        // other readers. Reads within a write transaction stay transaction-backed for read-your-writes.
        // See CONTEXT.md (read view).
        Ok(self.read_view())
    }

    fn create_write_tx(&self) -> Result<Self::WriteTransaction<'_>, StorageError> {
        let timer = Instant::now();
        let tx = self.db.transaction();
        let tx = RocksDbStateStoreWriteTransaction::new(&self.db, tx, &self.options);
        let elapsed = timer.elapsed();
        let level = if elapsed > Duration::from_secs(1) {
            log::Level::Warn
        } else {
            log::Level::Trace
        };
        log!(
            target: LOG_TARGET,
            level,
            "Write transaction obtained in {:?}", elapsed
        );
        Ok(tx)
    }
}

impl<TAddr, DB> Clone for RocksDbStateStore<TAddr, DB> {
    fn clone(&self) -> Self {
        Self {
            db: self.db.clone(),
            _addr: PhantomData,
            options: self.options.clone(),
            memory_budget: self.memory_budget.clone(),
        }
    }
}
