//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! What this node's configured caps add up to, checked against the machine it is starting on.
//!
//! Every term here is a limit the node enforces: a queue that drops beyond its byte budget, a cache
//! that evicts at its capacity, an interpreter whose instances are bounded by the engine limits.
//! That makes the sum the figure to size a machine from, and a machine that cannot supply it
//! misconfigured at startup rather than killed by the OOM killer at some later moment under load.
//!
//! Not every term is enforced the same way. The queues drop on arrival, so they are hard. The state
//! store enforces by triggering flushes and evicting, so it is a soft ceiling: memtable memory can
//! run past its share by as much as the buffers already in flight while those flushes complete, and
//! an LRU cache admits before it evicts. The overshoot is bounded — it is a multiple of
//! `write_buffer_bytes`, not of traffic — but "cap" here means "the level enforcement acts at", not
//! "a level allocation cannot reach".
//!
//! Terms that scale with traffic rather than with a cap — libp2p's message caches, the working set
//! of a block mid-execution, allocator fragmentation — are deliberately absent. Including a guess
//! here would make the check fail for reasons the node cannot control, so [`HEADROOM_FACTOR`]
//! covers them in one place instead.
//!
//! The budget itself is arithmetic over the configuration and is reported everywhere. The
//! comparison against the machine needs `/proc/meminfo`, so on a non-Linux build the budget is
//! logged and the comparison is skipped, with a line saying so.

use std::fmt::Write;

use log::*;
use tari_engine_types::limits::{ENGINE_LIMITS, WASM_LIMITS};
use tari_ootle_template_provider::TemplateConfig;
use tari_state_store_rocksdb::DatabaseOptions;

use crate::{config::ValidatorNodeConfig, p2p::services::mempool::MEM_MAX_TRANSACTIONS_DEDUP};

const LOG_TARGET: &str = "tari::validator_node::memory_budget";

/// Multiplier applied to the enforced caps to arrive at what the machine must actually have free.
///
/// It covers the terms this model deliberately does not enumerate — libp2p's message and peer
/// caches, the substate diffs held while a block executes, and the allocator arenas a long-running
/// process does not return to the OS once its queues have been full — and the overshoot the softly
/// enforced terms allow.
///
/// `tari-vn-bench` enumerates those unlisted terms instead of folding them into a factor, so its
/// table has more lines and a smaller multiplier. The two are different views of one model and move
/// together.
const HEADROOM_FACTOR: f64 = 1.5;

/// Bytes a transaction id occupies in the mempool's dedup set: 33 bytes across both generations it
/// keeps, plus the hash set's bucket overhead.
const DEDUP_BYTES_PER_TRANSACTION: u64 = 67;

/// One term of the budget.
pub struct BudgetLine {
    pub name: &'static str,
    pub bytes: u64,
}

/// The sum of every cap this node enforces, and the terms it is made of.
pub struct MemoryBudget {
    pub lines: Vec<BudgetLine>,
    /// Sum of the lines.
    pub capped_bytes: u64,
    /// `capped_bytes` grown by [`HEADROOM_FACTOR`]: what the machine must have free.
    pub required_bytes: u64,
}

impl MemoryBudget {
    /// Derives the budget from the configuration this node is about to start with.
    pub fn from_config(config: &ValidatorNodeConfig, db_options: &DatabaseOptions, templates: &TemplateConfig) -> Self {
        let lines = vec![
            BudgetLine {
                name: "Consensus gossip queue",
                bytes: config.max_consensus_gossip_queue_bytes as u64,
            },
            BudgetLine {
                name: "Transaction gossip queue",
                bytes: config.max_transaction_gossip_queue_bytes as u64,
            },
            BudgetLine {
                name: "Consensus messaging queue",
                bytes: config.max_consensus_messaging_queue_bytes as u64,
            },
            BudgetLine {
                name: "State store block cache and memtables",
                bytes: db_options.memory_budget_bytes as u64,
            },
            BudgetLine {
                name: "Compiled template module cache",
                bytes: templates.max_cache_size_bytes(),
            },
            BudgetLine {
                name: "Mempool dedup cache",
                bytes: MEM_MAX_TRANSACTIONS_DEDUP as u64 * DEDUP_BYTES_PER_TRANSACTION,
            },
            BudgetLine {
                // Block execution is serial, so one instance chain exists at a time; its depth is
                // what the engine limits allow, not what any single transaction requests.
                name: "WASM instance memory at maximum call depth",
                bytes: WASM_LIMITS.max_memory_pages as u64 * 64 * 1024 * ENGINE_LIMITS.max_call_depth as u64,
            },
        ];

        let capped_bytes = lines.iter().map(|l| l.bytes).sum::<u64>();
        let required_bytes = (capped_bytes as f64 * HEADROOM_FACTOR) as u64;

        Self {
            lines,
            capped_bytes,
            required_bytes,
        }
    }
}

/// Logs the budget and compares it against what the machine has free.
///
/// A shortfall is reported and startup continues: the caps are ceilings reached under load or
/// attack, not steady state, so a node below the requirement usually runs — until the day it does
/// not. Refusing to start would turn a machine that has been serving fine into one that will not
/// come back up after a restart.
pub fn check_against_available_memory(budget: &MemoryBudget) {
    let mut table = String::new();
    for line in &budget.lines {
        let _ = writeln!(table, "  {:>10}  {}", format_bytes(line.bytes), line.name);
    }
    let _ = writeln!(table, "  {:>10}  enforced caps", format_bytes(budget.capped_bytes));
    let _ = write!(
        table,
        "  {:>10}  required, including headroom for traffic-scaled buffers and allocator overhead",
        format_bytes(budget.required_bytes)
    );
    info!(target: LOG_TARGET, "Memory budget:\n{table}");

    let Some(available) = available_memory_bytes() else {
        info!(
            target: LOG_TARGET,
            "Available memory could not be read on {}, so the budget above is reported but not checked against this \
             machine. Confirm by hand that it has {} free.",
            std::env::consts::OS,
            format_bytes(budget.required_bytes),
        );
        return;
    };

    if available < budget.required_bytes {
        warn!(
            target: LOG_TARGET,
            "⚠️ This machine has {} available but the configured memory budget requires {}. The node will run under \
             ordinary load, but a burst that fills its queues can exhaust memory. Either provision more memory or \
             lower `state_store_memory_budget_bytes` and the `max_*_queue_bytes` settings in the configuration.",
            format_bytes(available),
            format_bytes(budget.required_bytes),
        );
    }
}

/// Memory the kernel believes can be handed out without swapping, read from `/proc/meminfo`.
///
/// `None` anywhere `/proc` is absent, which the node is built for but not deployed on. That is not
/// an error: the budget is arithmetic over the configuration and stands on its own, and only the
/// comparison against this machine is lost.
fn available_memory_bytes() -> Option<u64> {
    let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
    meminfo
        .lines()
        .find(|line| line.starts_with("MemAvailable:"))
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|kib| kib.parse::<u64>().ok())
        .map(|kib| kib * 1024)
}

fn format_bytes(bytes: u64) -> String {
    const MIB: f64 = 1024.0 * 1024.0;
    let mib = bytes as f64 / MIB;
    if mib >= 1024.0 {
        format!("{:.1} GiB", mib / 1024.0)
    } else {
        format!("{mib:.0} MiB")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn budget_of(config: &ValidatorNodeConfig, db_options: &DatabaseOptions) -> u64 {
        MemoryBudget::from_config(config, db_options, &TemplateConfig::default()).capped_bytes
    }

    /// Every configured cap must reach the total. Moving each input in turn and requiring the total
    /// to move with it is what catches a cap that was added to the configuration but never given a
    /// line here — the failure this table exists to prevent.
    #[test]
    fn every_configured_cap_reaches_the_total() {
        const DELTA: usize = 16 * 1024 * 1024;
        let base_config = ValidatorNodeConfig::default();
        let base_options = DatabaseOptions::default();
        let base = budget_of(&base_config, &base_options);

        type Mutation = fn(&mut ValidatorNodeConfig, &mut DatabaseOptions);

        let mutations: [(&str, Mutation); 4] = [
            ("consensus gossip queue", |c, _| {
                c.max_consensus_gossip_queue_bytes += DELTA
            }),
            ("transaction gossip queue", |c, _| {
                c.max_transaction_gossip_queue_bytes += DELTA
            }),
            ("consensus messaging queue", |c, _| {
                c.max_consensus_messaging_queue_bytes += DELTA
            }),
            ("state store budget", |_, o| o.memory_budget_bytes += DELTA),
        ];

        for (name, mutate) in mutations {
            let mut config = base_config.clone();
            let mut options = base_options.clone();
            mutate(&mut config, &mut options);
            assert_eq!(
                budget_of(&config, &options) - base,
                DELTA as u64,
                "{name} does not reach the total"
            );
        }
    }

    /// The caps that are constants rather than configuration cannot be varied, so they are checked
    /// by the value they contribute instead.
    #[test]
    fn the_constant_caps_are_present() {
        let templates = TemplateConfig::default();
        let budget =
            MemoryBudget::from_config(&ValidatorNodeConfig::default(), &DatabaseOptions::default(), &templates);

        for (name, bytes) in [
            ("template module cache", templates.max_cache_size_bytes()),
            (
                "mempool dedup cache",
                MEM_MAX_TRANSACTIONS_DEDUP as u64 * DEDUP_BYTES_PER_TRANSACTION,
            ),
            (
                "WASM instance memory",
                WASM_LIMITS.max_memory_pages as u64 * 64 * 1024 * ENGINE_LIMITS.max_call_depth as u64,
            ),
        ] {
            assert!(
                budget.lines.iter().any(|l| l.bytes == bytes),
                "{name} is not among the lines"
            );
        }
    }

    #[test]
    fn the_requirement_exceeds_the_caps_it_is_derived_from() {
        let budget = MemoryBudget::from_config(
            &ValidatorNodeConfig::default(),
            &DatabaseOptions::default(),
            &TemplateConfig::default(),
        );
        assert!(budget.required_bytes > budget.capped_bytes);
    }
}
