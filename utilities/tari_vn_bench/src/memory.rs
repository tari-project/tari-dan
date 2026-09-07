//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! The validator node's memory ceiling.
//!
//! Unlike throughput, this is not something a benchmark can measure by running faster or slower:
//! the ceiling is what the node allocates when every bounded buffer is simultaneously full, and a
//! quiet machine never reaches it. So it is *derived* — from the caps the code actually enforces —
//! and then, when a node is running, checked against what that node has really used.
//!
//! The distinction the table draws is the important one:
//!
//! * **Capped** terms have a hard limit in the code. A flood cannot push them past it; they are the part of the ceiling
//!   that is genuinely a ceiling.
//! * **Estimated** terms are bounded only by defaults or by traffic. They are where a memory surprise comes from, so
//!   they are listed with what they scale with rather than folded silently into a total.
//!
//! The headline number is the sum of both, because an operator has to provision for the sum. But a
//! node sitting far below it is normal, not evidence the model is wrong: the capped terms are
//! attack-and-burst ceilings, not steady state.

use serde::{Deserialize, Serialize};
use tari_engine_types::limits::{ENGINE_LIMITS, WASM_LIMITS};

const MIB: u64 = 1024 * 1024;

/// Whether a budget line is enforced by the code or merely expected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Bound {
    /// A limit the node enforces. Traffic cannot exceed it.
    Capped,
    /// Governed by a library default or by traffic volume, not by an explicit cap.
    Estimated,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetLine {
    pub name: String,
    pub bytes: u64,
    pub bound: Bound,
    /// Where the figure comes from, so a reader can re-derive it rather than trust it.
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryBudget {
    pub lines: Vec<BudgetLine>,
    /// Sum of the [`Bound::Capped`] lines: the part of the ceiling the code guarantees.
    pub capped_bytes: u64,
    /// Sum of the [`Bound::Estimated`] lines.
    pub estimated_bytes: u64,
    /// Capped plus estimated, then grown by [`ALLOCATOR_OVERHEAD`]. What a machine must have
    /// available to the node before the OS and everything else on the box.
    pub upper_bound_bytes: u64,
    /// What a live node has actually used, when one was pointed at with `--vn-pid`.
    pub observed: Option<ObservedProcess>,
}

/// Fragmentation and allocator arenas on top of the accounted terms. A long-running process that
/// has once filled its queues does not return every page to the OS, so the resident set settles
/// above the sum of live allocations.
const ALLOCATOR_OVERHEAD: f64 = 1.25;

/// Resident set of a validator node process, read from `/proc`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservedProcess {
    pub pid: u32,
    /// Current resident set.
    pub rss_bytes: Option<u64>,
    /// High-water mark since the process started. This is the number to compare against the
    /// budget: it survives the burst that current RSS has already forgotten.
    pub peak_rss_bytes: Option<u64>,
    /// Proportional set size, which charges shared pages only once. Closer to the node's true cost
    /// on a box running more than one process.
    pub pss_bytes: Option<u64>,
    pub uptime_hint: Option<String>,
}

/// Builds the budget for a node running with stock configuration.
///
/// Every figure is either imported from the crate that enforces it or carries the file that sets
/// it. The hardcoded ones live behind private defaults or inside RocksDB, so they cannot be
/// imported — when one of those moves, this table has to move with it.
pub fn budget(pid: Option<u32>) -> MemoryBudget {
    // Inbound queue caps. Sized in bytes precisely because a single gossip message may be up to the
    // swarm's 2 MiB `gossip_sub_max_message_size`, so these are reached by a flood of large
    // messages, not by ordinary traffic.
    let lines = vec![
        BudgetLine {
            name: "Consensus gossip queue".to_string(),
            bytes: 256 * MIB,
            bound: Bound::Capped,
            source: "tari_validator_node config default_max_consensus_gossip_queue_bytes".to_string(),
        },
        BudgetLine {
            name: "Transaction gossip queue".to_string(),
            bytes: 128 * MIB,
            bound: Bound::Capped,
            source: "tari_validator_node config default_max_transaction_gossip_queue_bytes".to_string(),
        },
        BudgetLine {
            name: "Consensus messaging queue".to_string(),
            bytes: 128 * MIB,
            bound: Bound::Capped,
            source: "tari_validator_node config default_max_consensus_messaging_queue_bytes".to_string(),
        },
        BudgetLine {
            name: "Mempool dedup cache".to_string(),
            bytes: 64 * MIB,
            bound: Bound::Capped,
            // 1e6 ids x 33 bytes across two generations, rounded up to power-of-two bucket counts.
            source: "mempool MEM_MAX_TRANSACTIONS_DEDUP = 1_000_000".to_string(),
        },
        BudgetLine {
            name: "WASM instance memory (worst-case call stack)".to_string(),
            bytes: (WASM_LIMITS.max_memory_pages as u64) * 64 * 1024 * (ENGINE_LIMITS.max_call_depth as u64),
            bound: Bound::Capped,
            source: "WASM_LIMITS.max_memory_pages x ENGINE_LIMITS.max_call_depth".to_string(),
        },
        // RocksDB is opened without an explicit write buffer size or shared block cache, so its
        // memory is whatever the library defaults to, multiplied by the column family count. This
        // is the largest single term in the model and the one least visible from the Ootle code.
        BudgetLine {
            name: "RocksDB memtables (9 column families)".to_string(),
            bytes: 9 * 64 * MIB * 2,
            bound: Bound::Estimated,
            source: "9 CFs x write_buffer_size 64 MiB x max_write_buffer_number 2 (rocksdb options.h defaults); \
                     db_write_buffer_size is 0, so nothing caps the sum across column families"
                .to_string(),
        },
        BudgetLine {
            name: "RocksDB block cache and pinned index/filter blocks".to_string(),
            bytes: 288 * MIB,
            bound: Bound::Estimated,
            source: "9 CFs x the 32 MiB internal cache rocksdb creates when block_cache is unset (table.h); holds \
                     index and filter blocks, which cache_index_and_filter_blocks puts there"
                .to_string(),
        },
        BudgetLine {
            name: "Gossipsub message cache and per-peer buffers".to_string(),
            bytes: 256 * MIB,
            bound: Bound::Estimated,
            // Scales with committee size and message size, neither of which the node caps directly.
            source: "libp2p gossipsub defaults x committee_size_per_shard_group x 2 MiB max message".to_string(),
        },
        BudgetLine {
            name: "Compiled template module cache".to_string(),
            bytes: 128 * MIB,
            bound: Bound::Estimated,
            // Grows with the number of distinct templates the shard group has seen; there is no
            // eviction, so this is the term that rises over a node's lifetime.
            source: "grows with published template count; ENGINE_LIMITS.max_template_binary_size_bytes each"
                .to_string(),
        },
        BudgetLine {
            name: "Block execution working set".to_string(),
            bytes: 128 * MIB,
            bound: Bound::Estimated,
            source: "max_commands_in_block substate diffs held while executing and committing a block".to_string(),
        },
    ];

    let capped_bytes = sum(&lines, Bound::Capped);
    let estimated_bytes = sum(&lines, Bound::Estimated);
    let upper_bound_bytes = (((capped_bytes + estimated_bytes) as f64) * ALLOCATOR_OVERHEAD) as u64;

    MemoryBudget {
        lines,
        capped_bytes,
        estimated_bytes,
        upper_bound_bytes,
        observed: pid.and_then(observe_process),
    }
}

fn sum(lines: &[BudgetLine], bound: Bound) -> u64 {
    lines.iter().filter(|l| l.bound == bound).map(|l| l.bytes).sum()
}

/// Reads a running node's footprint. Returns `None` if the pid does not exist or is not readable,
/// which is not an error — the budget stands on its own; the observation only corroborates it.
fn observe_process(pid: u32) -> Option<ObservedProcess> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    let rollup = std::fs::read_to_string(format!("/proc/{pid}/smaps_rollup")).unwrap_or_default();
    Some(ObservedProcess {
        pid,
        rss_bytes: kib_line(&status, "VmRSS"),
        peak_rss_bytes: kib_line(&status, "VmHWM"),
        pss_bytes: kib_line(&rollup, "Pss"),
        uptime_hint: std::fs::read_to_string(format!("/proc/{pid}/comm"))
            .ok()
            .map(|c| c.trim().to_string()),
    })
}

fn kib_line(text: &str, key: &str) -> Option<u64> {
    text.lines()
        .find(|line| line.starts_with(key) && line[key.len()..].starts_with(':'))
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|kib| kib.parse::<u64>().ok())
        .map(|kib| kib * 1024)
}
