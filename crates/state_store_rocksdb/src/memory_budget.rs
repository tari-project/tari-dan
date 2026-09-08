//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! The memory RocksDB is allowed to hold, expressed as one budget rather than as a per-column-family
//! default.
//!
//! Left alone, RocksDB sizes memtables per column family and creates an internal block cache per
//! options object, so the store's memory ceiling is the library's defaults multiplied by the number
//! of column families — a number nobody chose. [`RocksDbMemoryBudget`] replaces that with a single
//! [`Cache`] shared by every column family and a [`WriteBufferManager`] that charges memtable memory
//! to the same cache, so one capacity bounds reads and writes together.
//!
//! The budget also reports what it is currently holding, which is what makes the ceiling a
//! continuously checkable claim rather than a one-time calculation.

use rocksdb::{Cache, WriteBufferManager};

/// A shared block cache and the memtable budget charged against it.
///
/// Cloning is cheap and shares the same underlying cache: every clone reports and constrains the
/// same memory.
#[derive(Clone)]
pub struct RocksDbMemoryBudget {
    cache: Cache,
    write_buffer_manager: WriteBufferManager,
    capacity_bytes: usize,
    memtable_capacity_bytes: usize,
}

impl RocksDbMemoryBudget {
    /// Builds a budget of `capacity_bytes` total, of which memtables may occupy
    /// `memtable_capacity_bytes` before flushes are triggered.
    ///
    /// `memtable_capacity_bytes` is clamped to `capacity_bytes`: a write budget larger than the
    /// cache it is charged against would evict every cached block and still not be reached.
    pub fn new(capacity_bytes: usize, memtable_capacity_bytes: usize) -> Self {
        let memtable_capacity_bytes = memtable_capacity_bytes.min(capacity_bytes);
        let cache = Cache::new_lru_cache(capacity_bytes);
        // `allow_stall = false`: over budget triggers earlier flushes rather than blocking writers.
        // Stalling would make the bound hard, but a writer blocked inside a block commit can miss a
        // proposal, and enough missed proposals suspend the node — a memory bound must not be able
        // to cost liveness.
        let write_buffer_manager =
            WriteBufferManager::new_write_buffer_manager_with_cache(memtable_capacity_bytes, false, cache.clone());
        Self {
            cache,
            write_buffer_manager,
            capacity_bytes,
            memtable_capacity_bytes,
        }
    }

    pub(crate) fn cache(&self) -> &Cache {
        &self.cache
    }

    pub(crate) fn write_buffer_manager(&self) -> &WriteBufferManager {
        &self.write_buffer_manager
    }

    /// Total bytes the cache admits, memtable charges included.
    pub fn capacity_bytes(&self) -> usize {
        self.capacity_bytes
    }

    /// Bytes memtables may occupy before RocksDB starts flushing.
    pub fn memtable_capacity_bytes(&self) -> usize {
        self.memtable_capacity_bytes
    }

    /// Bytes currently held by live memtables across every column family.
    pub fn memtable_bytes(&self) -> usize {
        self.write_buffer_manager.get_usage()
    }

    /// Bytes currently charged to the cache: cached blocks, index and filter blocks, and the
    /// reservation covering `memtable_bytes`.
    pub fn cache_bytes(&self) -> usize {
        self.cache.get_usage()
    }

    /// The part of `cache_bytes` that cannot be evicted, which is where
    /// `pin_l0_filter_and_index_blocks_in_cache` shows up.
    pub fn cache_pinned_bytes(&self) -> usize {
        self.cache.get_pinned_usage()
    }
}

impl std::fmt::Debug for RocksDbMemoryBudget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RocksDbMemoryBudget")
            .field("capacity_bytes", &self.capacity_bytes)
            .field("memtable_capacity_bytes", &self.memtable_capacity_bytes)
            .field("cache_bytes", &self.cache_bytes())
            .field("memtable_bytes", &self.memtable_bytes())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memtable_budget_cannot_exceed_the_cache_it_is_charged_against() {
        let budget = RocksDbMemoryBudget::new(64 * 1024 * 1024, 128 * 1024 * 1024);
        assert_eq!(budget.memtable_capacity_bytes(), 64 * 1024 * 1024);
    }

    /// Sharing is the whole point: the budget handed to one column family's options must constrain
    /// and report the same memory as every other's. Writing through one handle and reading usage
    /// through another is what distinguishes sharing from a copy that merely reports equal figures.
    #[test]
    fn a_clone_observes_writes_made_through_the_original() {
        let budget = RocksDbMemoryBudget::new(64 * 1024 * 1024, 32 * 1024 * 1024);
        let observer = budget.clone();
        assert_eq!(observer.memtable_bytes(), 0);

        let temp = tempfile::tempdir().unwrap();
        let mut opts = rocksdb::Options::default();
        opts.create_if_missing(true);
        opts.set_write_buffer_manager(budget.write_buffer_manager());
        let db = rocksdb::DB::open(&opts, temp.path().join("db")).unwrap();
        db.put(b"key", vec![0u8; 4 * 1024 * 1024]).unwrap();

        assert!(
            observer.memtable_bytes() > 0,
            "the clone reports no memtable memory, so it is not charged against the same budget"
        );
    }
}
