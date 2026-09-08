//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

/// Total memory RocksDB may hold across its block cache and its memtables.
///
/// Memtable memory is charged to the block cache, so this single figure bounds both: block, index
/// and filter data is evicted to make room for memtables rather than the two growing side by side.
pub const DEFAULT_MEMORY_BUDGET_BYTES: usize = 768 * 1024 * 1024;

/// The share of [`DEFAULT_MEMORY_BUDGET_BYTES`] memtables may occupy before RocksDB starts
/// flushing. The remainder is what stays available to cache reads.
///
/// Half is RocksDB's own guidance: a write budget larger than that starves the read cache under
/// write-heavy load, and a much smaller one flushes so often that compaction never catches up.
pub const DEFAULT_MEMTABLE_BUDGET_BYTES: usize = DEFAULT_MEMORY_BUDGET_BYTES / 2;

/// The size of a single column family's active memtable.
///
/// The memtable budget is enforced by triggering flushes, not by blocking writers, so the amount
/// by which memtable memory can overshoot the budget is bounded by the buffers already in flight.
/// Smaller per-family buffers make that overshoot smaller.
pub const DEFAULT_WRITE_BUFFER_BYTES: usize = 32 * 1024 * 1024;

/// Memtables a column family may hold at once: the active one plus one being flushed.
///
/// With [`DEFAULT_WRITE_BUFFER_BYTES`] and the column family count, this is what bounds the
/// overshoot above: no column family can hold more than this many buffers, so nothing can hold
/// more than their product while flushes complete.
pub const MAX_WRITE_BUFFER_NUMBER: i32 = 2;

#[derive(Debug, Clone)]
pub struct DatabaseOptions {
    /// The versions behind the latest to keep for each shard.
    /// The default is 100. This preserves the last 100 versions of the state tree i.e. if the last 100 blocks all have
    /// state transitions, then we preserve 100 blocks worth of deleted state. Currently, this only applies to the
    /// state tree stale nodes.
    pub state_history_length: u64,
    /// The number of epochs back from the current epoch to keep in the database.
    /// This includes blocks, foreign proposals etc.
    /// The default is 1, which means we keep the previous epoch's data until this epoch has passed. It is not
    /// recommended to set this to 0.
    pub epoch_history_length: u64,
    /// Whether to store additional debugging data in the database. This may increase storage requirements and slow
    /// down some operations, so it should only be enabled for debugging purposes.
    pub debugging_data: bool,
    /// Whether epoch GC prunes finalized transaction bookkeeping (payloads, results and finalized
    /// markers) once it ages past `epoch_history_length`. Disable to retain the full transaction
    /// history (an archival node). Chain state — committed effects and transaction receipts — is
    /// retained regardless, so consensus behaviour is identical either way. The epoch index that
    /// drives pruning is always maintained, so pruning can be enabled later and will still remove
    /// history accumulated while it was disabled.
    pub prune_transaction_history: bool,
    /// Total bytes RocksDB may hold across its block cache and memtables, shared by every column
    /// family. See [`DEFAULT_MEMORY_BUDGET_BYTES`].
    pub memory_budget_bytes: usize,
    /// The portion of `memory_budget_bytes` memtables may occupy before flushes are triggered.
    /// Must not exceed `memory_budget_bytes`. See [`DEFAULT_MEMTABLE_BUDGET_BYTES`].
    pub memtable_budget_bytes: usize,
    /// The size of one column family's active memtable. See [`DEFAULT_WRITE_BUFFER_BYTES`].
    pub write_buffer_bytes: usize,
}

impl DatabaseOptions {
    /// Whether to store additional debugging data in the database. This may increase storage requirements and slow
    /// down some operations, so it should only be enabled for debugging purposes.
    pub fn with_debugging_data(mut self, debugging_data: bool) -> Self {
        self.debugging_data = debugging_data;
        self
    }

    /// The versions behind the latest to keep for each shard.
    /// The default is 100. This preserves the last 100 versions of the state tree i.e. if the last 100 blocks all have
    /// state transitions, then we preserve 100 blocks worth of deleted state. Currently, this only applies to the
    /// state tree stale nodes.
    pub fn with_state_history_length(mut self, state_history_length: u64) -> Self {
        self.state_history_length = state_history_length;
        self
    }

    /// The number of epochs back from the current epoch to keep in the database.
    /// This includes blocks, foreign proposals etc.
    /// The default is 1, which means we keep the previous epoch's data until this epoch has passed. It is not
    /// recommended to set this to 0.
    pub fn with_epoch_history_length(mut self, epoch_history_length: u64) -> Self {
        self.epoch_history_length = epoch_history_length;
        self
    }

    /// Whether epoch GC prunes finalized transaction bookkeeping once it ages past
    /// `epoch_history_length`. Disable to retain the full transaction history (an archival node).
    pub fn with_prune_transaction_history(mut self, prune_transaction_history: bool) -> Self {
        self.prune_transaction_history = prune_transaction_history;
        self
    }

    /// Total bytes RocksDB may hold across its block cache and memtables.
    pub fn with_memory_budget_bytes(mut self, memory_budget_bytes: usize) -> Self {
        self.memory_budget_bytes = memory_budget_bytes;
        self
    }

    /// The portion of the memory budget memtables may occupy before flushes are triggered.
    pub fn with_memtable_budget_bytes(mut self, memtable_budget_bytes: usize) -> Self {
        self.memtable_budget_bytes = memtable_budget_bytes;
        self
    }

    /// The size of one column family's active memtable, which bounds how far memtable memory can
    /// overshoot the budget while triggered flushes are in flight.
    pub fn with_write_buffer_bytes(mut self, write_buffer_bytes: usize) -> Self {
        self.write_buffer_bytes = write_buffer_bytes;
        self
    }
}

impl Default for DatabaseOptions {
    fn default() -> Self {
        Self {
            state_history_length: 100,
            epoch_history_length: 1,
            debugging_data: false,
            prune_transaction_history: true,
            memory_budget_bytes: DEFAULT_MEMORY_BUDGET_BYTES,
            memtable_budget_bytes: DEFAULT_MEMTABLE_BUDGET_BYTES,
            write_buffer_bytes: DEFAULT_WRITE_BUFFER_BYTES,
        }
    }
}
