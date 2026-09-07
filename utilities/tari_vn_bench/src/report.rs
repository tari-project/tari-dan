//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Turns measurements into a verdict.
//!
//! The bar is not "the node starts". It is "the node keeps voting": a validator that cannot execute
//! and vote inside `pacemaker_block_time` misses proposals, and missing
//! `missed_proposal_suspend_threshold` of them suspends it while
//! `missed_proposal_evict_threshold` evicts it. Every threshold below is derived from that, against
//! the consensus constants for the network being sized for, so a re-tuned constant re-tunes the
//! verdict with it.
//!
//! Grades are deliberately coarse. An operator's decision is binary — provision this machine or
//! don't — and the numbers that produced the grade are printed alongside it for anyone who wants to
//! argue with the thresholds.

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use tari_consensus::consensus_constants::ConsensusConstants;
use tari_engine_types::limits::NativeExecutionPoints;

use crate::{
    capacity::CapacityProjection,
    execution::ExecutionMeasurement,
    host::Host,
    memory::{Bound, MemoryBudget},
    native::NativeMeasurement,
    storage::StorageMeasurement,
    wire::WireMeasurement,
};

/// Weight per second measured on the two-core class the block weight budgets were calibrated
/// against. A machine below this is slower than the hardware the constants assume, which is the
/// clearest possible statement that it is under-specified.
const REFERENCE_WEIGHT_PER_SEC: f64 = 2_700.0;

/// Metered points per millisecond the block execution-point budgets are calibrated against.
const REFERENCE_POINTS_PER_MS: f64 = 8_400_000.0;

/// Native points charged for the canonical stealth transfer the native phase times: one statement,
/// two outputs, one input, no view key. Compared against the wall clock that shape really takes, it
/// says whether native verification is priced correctly *on this host*.
const CANONICAL_STEALTH_POINTS: u64 =
    NativeExecutionPoints::PER_STATEMENT + 2 * NativeExecutionPoints::PER_OUTPUT + NativeExecutionPoints::PER_INPUT;

/// Ratio of measured to priced native verification time above which the block execution-point
/// budget systematically understates what a stealth-heavy block costs this machine. Set with slack:
/// the price is a network-wide constant fitted on other hardware, so some spread is expected and
/// only a large skew is a sizing problem.
const NATIVE_SKEW_WARN: f64 = 1.5;

/// Verifications per second below which mempool admission cannot keep up with the transaction rate
/// blocks alone imply (`max_commands_in_block` per block time), let alone the gossip that feeds it.
const SIGNATURE_VERIFY_FLOOR_MULTIPLE: f64 = 10.0;

/// fsync p99 thresholds. A commit syncs the write-ahead log, and epoch GC and state sync issue many
/// in a burst, so the tail is what matters rather than the median.
const FSYNC_P99_PASS_MS: f64 = 2.0;
const FSYNC_P99_WARN_MS: f64 = 10.0;

/// Sustained sequential write thresholds. This governs state sync and compaction — how long a node
/// takes to join a shard group or recover — rather than per-view latency.
const SEQ_WRITE_PASS_MIB: f64 = 100.0;
const SEQ_WRITE_WARN_MIB: f64 = 20.0;

/// Memory the OS, logging and everything else on the box need beyond the node's own ceiling.
const OS_HEADROOM_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Grade {
    Pass,
    Warn,
    Fail,
}

impl Grade {
    fn label(self) -> &'static str {
        match self {
            Grade::Pass => "PASS",
            Grade::Warn => "WARN",
            Grade::Fail => "FAIL",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub axis: String,
    pub grade: Grade,
    pub headline: String,
    pub detail: String,
}

/// The consensus constants every projection is made against, recorded so a report stays readable
/// after the constants move.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Budgets {
    pub network: String,
    pub block_time_secs: f64,
    /// Half the block time: the point at which `on_propose` stops executing and defers the rest of
    /// the batch to a later block.
    pub propose_exec_deadline_secs: f64,
    pub max_block_weight: u64,
    pub max_block_validation_weight: u64,
    pub max_block_execution_points: u64,
    pub max_block_validation_execution_points: u64,
    pub max_commands_in_block: usize,
    pub committee_size_per_shard_group: u32,
    pub missed_proposal_suspend_threshold: u64,
    pub missed_proposal_evict_threshold: u64,
}

impl Budgets {
    pub fn new(network: &str, constants: &ConsensusConstants) -> Self {
        Self {
            network: network.to_string(),
            block_time_secs: constants.pacemaker_block_time.as_secs_f64(),
            propose_exec_deadline_secs: constants.pacemaker_block_time.as_secs_f64() / 2.0,
            max_block_weight: constants.max_block_weight,
            max_block_validation_weight: constants.max_block_validation_weight,
            max_block_execution_points: constants.max_block_execution_points,
            max_block_validation_execution_points: constants.max_block_validation_execution_points,
            max_commands_in_block: constants.max_commands_in_block,
            committee_size_per_shard_group: constants.committee_size_per_shard_group,
            missed_proposal_suspend_threshold: constants.missed_proposal_suspend_threshold,
            missed_proposal_evict_threshold: constants.missed_proposal_evict_threshold,
        }
    }
}

/// What the measurements say a full block costs this machine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Projections {
    /// Executing a full `max_block_weight` block — what this node does on the views it leads.
    pub propose_block_secs: f64,
    /// Executing a full `max_block_validation_weight` block — the worst block a leader can make
    /// this node execute before it may vote.
    pub validation_block_secs: f64,
    /// The same worst case expressed through the execution-point budget rather than weight.
    pub validation_points_secs: f64,
    /// Measured weight/s as a multiple of the class the constants were calibrated on.
    pub weight_rate_vs_reference: f64,
    pub points_rate_vs_reference: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub tool_version: String,
    pub host: Host,
    pub budgets: Budgets,
    pub execution: Option<ExecutionMeasurement>,
    pub native: Option<NativeMeasurement>,
    pub storage: Option<StorageMeasurement>,
    /// Why the storage phase produced nothing, when it was attempted and failed. Kept separate from
    /// `storage: None`, which also covers `--skip-storage`: a phase the operator declined is not the
    /// same as one that broke, and only the second needs to appear in the verdict.
    #[serde(default)]
    pub storage_error: Option<String>,
    pub memory: MemoryBudget,
    /// Encoded sizes of blocks and transactions. Deterministic and identical on every host, so it
    /// is reported rather than graded.
    pub wire: WireMeasurement,
    /// Bandwidth and disk requirements derived from `wire` and the consensus constants. Properties
    /// of the network, not of this machine, and likewise not graded.
    pub capacity: CapacityProjection,
    pub projections: Option<Projections>,
    pub findings: Vec<Finding>,
    pub grade: Grade,
}

impl Report {
    pub fn build(
        host: Host,
        budgets: Budgets,
        execution: Option<ExecutionMeasurement>,
        native: Option<NativeMeasurement>,
        storage: Option<StorageMeasurement>,
        storage_error: Option<String>,
        memory: MemoryBudget,
        wire: WireMeasurement,
        capacity: CapacityProjection,
    ) -> Self {
        let projections = execution.as_ref().map(|e| project(e, &budgets));
        let mut findings = Vec::new();

        if host.debug_assertions {
            findings.push(Finding {
                axis: "build".to_string(),
                grade: Grade::Fail,
                headline: "Benchmark was built without optimisations".to_string(),
                detail: "A debug build runs the engine several times slower than the release build a validator \
                         actually runs, so every execution figure below understates this machine by a wide margin. \
                         Rebuild with --release and re-run."
                    .to_string(),
            });
        }

        if let (Some(execution), Some(projections)) = (execution.as_ref(), projections.as_ref()) {
            findings.push(grade_execution_weight(execution, projections, &budgets));
            findings.push(grade_execution_points(execution, projections, &budgets));
        }
        if let Some(native) = native.as_ref() {
            findings.push(grade_signature_verification(native, &budgets));
            if let Some(execution) = execution.as_ref() {
                findings.push(grade_native_skew(native, execution));
            }
        }
        if let Some(storage) = storage.as_ref() {
            findings.push(grade_storage(storage));
        }
        if let Some(error) = storage_error.as_ref() {
            findings.push(Finding {
                axis: "storage".to_string(),
                grade: Grade::Warn,
                headline: "Storage was not measured".to_string(),
                detail: format!(
                    "{error} The rest of this report stands, but storage is unassessed — fsync latency floors every \
                     block commit, so a machine can pass every other axis and still miss proposals on a slow volume. \
                     Re-run with --data-dir pointing at a writable path on the volume the node's data directory will \
                     use."
                ),
            });
        }
        findings.push(grade_memory(&host, &memory));

        let grade = findings.iter().map(|f| f.grade).max().unwrap_or(Grade::Pass);

        Self {
            tool_version: env!("CARGO_PKG_VERSION").to_string(),
            host,
            budgets,
            execution,
            native,
            storage,
            storage_error,
            memory,
            wire,
            capacity,
            projections,
            findings,
            grade,
        }
    }
}

fn project(execution: &ExecutionMeasurement, budgets: &Budgets) -> Projections {
    let weight_per_sec = execution.block.weight_per_sec;
    let points_per_sec = execution.wasm_rate_points_per_ms * 1000.0;
    Projections {
        propose_block_secs: budgets.max_block_weight as f64 / weight_per_sec,
        validation_block_secs: budgets.max_block_validation_weight as f64 / weight_per_sec,
        validation_points_secs: budgets.max_block_validation_execution_points as f64 / points_per_sec,
        weight_rate_vs_reference: weight_per_sec / REFERENCE_WEIGHT_PER_SEC,
        points_rate_vs_reference: execution.wasm_rate_points_per_ms / REFERENCE_POINTS_PER_MS,
    }
}

/// The weight budget: can this machine execute a full block of ordinary transactions in time?
fn grade_execution_weight(execution: &ExecutionMeasurement, p: &Projections, b: &Budgets) -> Finding {
    let grade = if p.validation_block_secs >= b.block_time_secs || p.propose_block_secs >= b.propose_exec_deadline_secs
    {
        Grade::Fail
    } else if p.validation_block_secs > b.propose_exec_deadline_secs ||
        p.propose_block_secs > b.propose_exec_deadline_secs / 2.0
    {
        Grade::Warn
    } else {
        Grade::Pass
    };

    let headline = format!(
        "{:.0} weight/s — full block {:.2}s, worst valid block {:.2}s (block time {:.0}s)",
        execution.block.weight_per_sec, p.propose_block_secs, p.validation_block_secs, b.block_time_secs
    );

    let detail = match grade {
        Grade::Fail => format!(
            "A worst-case valid block ({} weight) takes {:.2}s to execute and a block this node proposes takes \
             {:.2}s, against a {:.0}s view and a {:.1}s propose deadline. This machine will miss proposals under load \
             — {} missed suspends it, {} evicts it.",
            b.max_block_validation_weight,
            p.validation_block_secs,
            p.propose_block_secs,
            b.block_time_secs,
            b.propose_exec_deadline_secs,
            b.missed_proposal_suspend_threshold,
            b.missed_proposal_evict_threshold,
        ),
        Grade::Warn => format!(
            "Execution alone consumes {:.0}% of a view on a worst-case block, leaving the rest for consensus, storage \
             and networking. It will keep up in normal traffic but has little margin under a deliberate overload.",
            p.validation_block_secs / b.block_time_secs * 100.0,
        ),
        Grade::Pass => format!(
            "Executes {:.1}x faster than the two-core class the weight budgets were calibrated against, and a \
             worst-case block leaves {:.2}s of the view unused.",
            p.weight_rate_vs_reference,
            b.block_time_secs - p.validation_block_secs,
        ),
    };

    // Stated on every grade, because it bounds how far any of them can be trusted.
    let detail = format!(
        "{detail} This figure is execution alone, against an in-memory state store. A node's propose and vote loops \
         also read and write substates, maintain the state tree and commit to RocksDB, so its own logged weight/s \
         will be lower — treat this as an upper bound and confirm against that log once the node runs."
    );

    Finding {
        axis: "cpu/weight".to_string(),
        grade,
        headline,
        detail,
    }
}

/// The execution-point budget: the compute-heavy counterpart, which transaction weight is blind to.
fn grade_execution_points(execution: &ExecutionMeasurement, p: &Projections, b: &Budgets) -> Finding {
    let grade = if p.validation_points_secs >= b.block_time_secs {
        Grade::Fail
    } else if p.validation_points_secs > b.propose_exec_deadline_secs {
        Grade::Warn
    } else {
        Grade::Pass
    };

    Finding {
        axis: "cpu/points".to_string(),
        grade,
        headline: format!(
            "{:.2}M points/ms — a full {}-point block projects to {:.2}s",
            execution.wasm_rate_points_per_ms / 1e6,
            b.max_block_validation_execution_points,
            p.validation_points_secs,
        ),
        detail: format!(
            "Marginal WASM execution rate, fixed per-transaction overhead cancelled. This is {:.1}x the {:.1}M \
             points/ms the block execution-point budgets are calibrated against.",
            p.points_rate_vs_reference,
            REFERENCE_POINTS_PER_MS / 1e6,
        ),
    }
}

/// Mempool admission: every gossiped transaction is verified before anything else happens to it.
fn grade_signature_verification(native: &NativeMeasurement, b: &Budgets) -> Finding {
    // Blocks alone imply this many verifications per second; gossip delivers far more, since a node
    // sees transactions it never sequences.
    let block_implied_per_sec = b.max_commands_in_block as f64 / b.block_time_secs;
    let comfortable = block_implied_per_sec * SIGNATURE_VERIFY_FLOOR_MULTIPLE;

    let grade = if native.signature_verifies_per_sec < block_implied_per_sec {
        Grade::Fail
    } else if native.signature_verifies_per_sec < comfortable {
        Grade::Warn
    } else {
        Grade::Pass
    };

    Finding {
        axis: "native/signatures".to_string(),
        grade,
        headline: format!(
            "{:.0} transaction signature verifications/s per core",
            native.signature_verifies_per_sec
        ),
        detail: format!(
            "Block content alone implies {block_implied_per_sec:.0}/s ({} commands per {:.0}s block); mempool \
             admission must clear well above that because it verifies transactions the node never sequences.",
            b.max_commands_in_block, b.block_time_secs,
        ),
    }
}

/// Whether native crypto is priced correctly *for this host*.
///
/// Native verification runs outside the WASM meter and is charged at a network-wide points price
/// fitted by wall-clock equivalence against the WASM rate. That price is only right on a machine
/// whose native-to-WASM ratio matches the one it was fitted on. Where it does not, a block full of
/// stealth statements costs more time than the execution-point budget believes — and the budget is
/// what stops a leader from overrunning replicas.
fn grade_native_skew(native: &NativeMeasurement, execution: &ExecutionMeasurement) -> Finding {
    let priced_ms = CANONICAL_STEALTH_POINTS as f64 / execution.wasm_rate_points_per_ms;
    let measured_ms = native.stealth_transfer_verify.min_ms;
    let skew = measured_ms / priced_ms;

    let grade = if skew > NATIVE_SKEW_WARN {
        Grade::Warn
    } else {
        Grade::Pass
    };

    Finding {
        axis: "native/stealth".to_string(),
        grade,
        headline: format!(
            "stealth transfer verifies in {measured_ms:.3}ms; priced at {priced_ms:.3}ms on this host ({skew:.2}x)"
        ),
        detail: if grade == Grade::Warn {
            format!(
                "Native crypto is {skew:.2}x slower than this machine's WASM rate implies, so the block \
                 execution-point budget understates what a stealth-heavy block costs it. Treat the points headroom \
                 above as optimistic for that traffic mix."
            )
        } else {
            "Native verification and WASM execution are in the ratio the native point prices assume, so the \
             execution-point budget describes this host's real cost."
                .to_string()
        },
    }
}

fn grade_storage(storage: &StorageMeasurement) -> Finding {
    let volatile = storage
        .filesystem
        .as_deref()
        .is_some_and(|fs| matches!(fs, "tmpfs" | "ramfs"));

    if volatile {
        return Finding {
            axis: "storage".to_string(),
            grade: Grade::Fail,
            headline: format!(
                "{} is {} — a RAM-backed filesystem",
                storage.path,
                storage.filesystem.as_deref().unwrap_or("unknown")
            ),
            detail: "These figures measure RAM, not a device, and a node's data directory here loses the whole \
                     database on reboot. Re-run against the volume the data directory will actually live on."
                .to_string(),
        };
    }

    let fsync_grade = if storage.fsync.p99_ms > FSYNC_P99_WARN_MS {
        Grade::Fail
    } else if storage.fsync.p99_ms > FSYNC_P99_PASS_MS {
        Grade::Warn
    } else {
        Grade::Pass
    };
    let write_grade = if storage.sequential_write_mib_per_sec < SEQ_WRITE_WARN_MIB {
        Grade::Fail
    } else if storage.sequential_write_mib_per_sec < SEQ_WRITE_PASS_MIB {
        Grade::Warn
    } else {
        Grade::Pass
    };

    Finding {
        axis: "storage".to_string(),
        grade: fsync_grade.max(write_grade),
        headline: format!(
            "fsync p50 {:.2}ms / p99 {:.2}ms, sequential write {:.0} MiB/s ({})",
            storage.fsync.p50_ms,
            storage.fsync.p99_ms,
            storage.sequential_write_mib_per_sec,
            storage.filesystem.as_deref().unwrap_or("unknown fs"),
        ),
        detail: "fsync latency is the floor under every block commit — RocksDB syncs its write-ahead log — and its \
                 tail is what a burst of epoch GC or state-sync writes runs into. Sequential throughput governs how \
                 long joining a shard group or recovering takes."
            .to_string(),
    }
}

fn grade_memory(host: &Host, memory: &MemoryBudget) -> Finding {
    let Some(total) = host.mem_total_bytes else {
        return Finding {
            axis: "memory".to_string(),
            grade: Grade::Warn,
            headline: "Host memory could not be read".to_string(),
            detail: format!(
                "The derived ceiling is {}; confirm by hand that this machine has that plus room for the OS.",
                human_bytes(memory.upper_bound_bytes)
            ),
        };
    };

    let required = memory.upper_bound_bytes + OS_HEADROOM_BYTES;
    let grade = if total < memory.capped_bytes + OS_HEADROOM_BYTES {
        Grade::Fail
    } else if total < required {
        Grade::Warn
    } else {
        Grade::Pass
    };

    let mut detail = format!(
        "{} of capped buffers plus {} of default- and traffic-governed allocation, grown for allocator overhead, \
         gives a {} ceiling; with {} for the OS this machine wants {}.",
        human_bytes(memory.capped_bytes),
        human_bytes(memory.estimated_bytes),
        human_bytes(memory.upper_bound_bytes),
        human_bytes(OS_HEADROOM_BYTES),
        human_bytes(required),
    );
    if let Some((observed, peak)) = memory
        .observed
        .as_ref()
        .and_then(|observed| observed.peak_rss_bytes.map(|peak| (observed, peak)))
    {
        let _ = write!(
            detail,
            " The running node (pid {}) has peaked at {}, {:.0}% of the ceiling.",
            observed.pid,
            human_bytes(peak),
            peak as f64 / memory.upper_bound_bytes as f64 * 100.0,
        );
    }

    Finding {
        axis: "memory".to_string(),
        grade,
        headline: format!(
            "{} total, {} ceiling for the node",
            human_bytes(total),
            human_bytes(memory.upper_bound_bytes)
        ),
        detail,
    }
}

pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Renders the human-readable report. Each section is written by its own helper so a section can be
/// changed without reading the rest.
pub fn render(report: &Report) -> String {
    let mut out = String::new();
    render_header(&mut out, report);
    if let (Some(execution), Some(projections)) = (&report.execution, &report.projections) {
        render_execution(&mut out, execution, projections);
    }
    if let Some(native) = &report.native {
        render_native(&mut out, native);
    }
    if let Some(storage) = &report.storage {
        render_storage(&mut out, storage);
    }
    render_memory(&mut out, &report.memory);
    render_capacity(&mut out, report);
    render_findings(&mut out, report);
    out
}

fn render_header(out: &mut String, report: &Report) {
    let host = &report.host;
    let _ = writeln!(out, "\ntari-vn-bench {} — {}", report.tool_version, host.label);
    let _ = writeln!(out, "{}", "=".repeat(78));
    let _ = writeln!(
        out,
        "Host      {} / {} — {} logical cpu(s){}{}",
        host.os,
        host.arch,
        host.logical_cpus,
        host.physical_cpus
            .map(|n| format!(", {n} physical core(s)"))
            .unwrap_or_default(),
        if host.smt_enabled() {
            " [SMT on: siblings add no single-thread speed]"
        } else {
            ""
        },
    );
    if let Some(model) = &host.cpu_model {
        let _ = writeln!(out, "CPU       {model}");
    }
    let _ = writeln!(
        out,
        "Memory    {} total, {} available",
        host.mem_total_bytes.map(human_bytes).unwrap_or("?".into()),
        host.mem_available_bytes.map(human_bytes).unwrap_or("?".into()),
    );
    let _ = writeln!(
        out,
        "Sizing    {} — {:.0}s block time, {:.1}s propose deadline, {} worst-case block weight",
        report.budgets.network,
        report.budgets.block_time_secs,
        report.budgets.propose_exec_deadline_secs,
        report.budgets.max_block_validation_weight,
    );
}

fn render_execution(out: &mut String, execution: &ExecutionMeasurement, projections: &Projections) {
    let _ = writeln!(
        out,
        "\nEXECUTION (in-memory state store; excludes storage and state-tree work)"
    );
    let _ = writeln!(out, "{}", "-".repeat(78));
    let _ = writeln!(
        out,
        "  builtin template compile     {:.0} ms (one-off at startup, and per template publish)",
        execution.template_compile_ms
    );
    let _ = writeln!(
        out,
        "  canonical transfer           {} weight, {} points, {:.2} ms (p99 {:.2} ms)",
        execution.transfer.weight,
        execution.transfer.points,
        execution.transfer.time.min_ms,
        execution.transfer.time.p99_ms,
    );
    let _ = writeln!(
        out,
        "  block of {:>4} transactions   {} weight in {:.3} s",
        execution.block.transactions, execution.block.total_weight, execution.block.elapsed_secs,
    );
    let _ = writeln!(
        out,
        "  throughput                   {:.0} weight/s ({:.1}x the calibration reference)",
        execution.block.weight_per_sec, projections.weight_rate_vs_reference,
    );
    let _ = writeln!(
        out,
        "  marginal WASM rate           {:.2}M points/ms ({:.1}x the calibration reference)",
        execution.wasm_rate_points_per_ms / 1e6,
        projections.points_rate_vs_reference,
    );
    if execution.transfer.time.dispersion() > 0.5 {
        let _ = writeln!(
            out,
            "  ! timings spread {:.0}% above their minimum — the machine was not idle; re-run before trusting the \
             margins",
            execution.transfer.time.dispersion() * 100.0,
        );
    }
}

fn render_native(out: &mut String, native: &NativeMeasurement) {
    let _ = writeln!(out, "\nNATIVE VERIFICATION");
    let _ = writeln!(out, "{}", "-".repeat(78));
    let _ = writeln!(
        out,
        "  transaction signatures       {:.3} ms  ({:.0}/s per core)",
        native.signature_verify.min_ms, native.signature_verifies_per_sec,
    );
    let _ = writeln!(
        out,
        "  stealth transfer (1in/2out)  {:.3} ms  ({:.0}/s per core)",
        native.stealth_transfer_verify.min_ms, native.stealth_verifies_per_sec,
    );
}

fn render_storage(out: &mut String, storage: &StorageMeasurement) {
    let _ = writeln!(out, "\nSTORAGE — {}", storage.path);
    let _ = writeln!(out, "{}", "-".repeat(78));
    let _ = writeln!(
        out,
        "  fsync                        min {:.2} / p50 {:.2} / p99 {:.2} / max {:.2} ms",
        storage.fsync.min_ms, storage.fsync.p50_ms, storage.fsync.p99_ms, storage.fsync.max_ms,
    );
    let _ = writeln!(
        out,
        "  sequential write             {:.0} MiB/s over {}",
        storage.sequential_write_mib_per_sec,
        human_bytes(storage.bytes_written),
    );
    let _ = writeln!(
        out,
        "  filesystem                   {}",
        storage.filesystem.as_deref().unwrap_or("unknown"),
    );
    if storage.fsync.dispersion() > 1.0 {
        let _ = writeln!(
            out,
            "  ! fsync p50 is {:.0}% above its minimum — the volume was not idle, so the tail below is the machine's \
             contention, not the device's floor",
            storage.fsync.dispersion() * 100.0,
        );
    }
}

fn render_memory(out: &mut String, memory: &MemoryBudget) {
    let _ = writeln!(out, "\nMEMORY CEILING (derived, not measured)");
    let _ = writeln!(out, "{}", "-".repeat(78));
    for line in &memory.lines {
        let _ = writeln!(
            out,
            "  {:<9} {:>10}  {}",
            match line.bound {
                Bound::Capped => "capped",
                Bound::Estimated => "estimated",
            },
            human_bytes(line.bytes),
            line.name,
        );
        for wrapped in wrap(&line.source, 58) {
            let _ = writeln!(out, "  {:<9} {:>10}    {wrapped}", "", "");
        }
    }
    let _ = writeln!(
        out,
        "  {:<9} {:>10}  capped subtotal",
        "",
        human_bytes(memory.capped_bytes)
    );
    let _ = writeln!(
        out,
        "  {:<9} {:>10}  ceiling including allocator overhead",
        "",
        human_bytes(memory.upper_bound_bytes)
    );
    if let Some(observed) = &memory.observed {
        let _ = writeln!(
            out,
            "  {:<9} {:>10}  observed peak RSS of pid {} ({})",
            "",
            observed.peak_rss_bytes.map(human_bytes).unwrap_or("?".into()),
            observed.pid,
            observed.uptime_hint.as_deref().unwrap_or("unknown process"),
        );
    }
}

/// Network-wide requirements. Printed after the host sections and deliberately outside the verdict:
/// these numbers are identical on every validator, so grading this machine against them would say
/// nothing about this machine.
fn render_capacity(out: &mut String, report: &Report) {
    let wire = &report.wire;
    let cap = &report.capacity;

    let _ = writeln!(out, "\nENCODED SIZES (measured; same on every host)");
    let _ = writeln!(out, "{}", "-".repeat(78));
    let _ = writeln!(
        out,
        "  canonical transfer           {} bytes on the wire",
        wire.transaction_bytes
    );
    for (committees, bytes) in &wire.command_bytes_by_committees {
        let _ = writeln!(
            out,
            "  block command                {bytes} bytes at {committees} committee(s)"
        );
    }
    let _ = writeln!(
        out,
        "  full block of transfers      {} ({} commands at {} weight each — weight-bound)",
        human_bytes(wire.block_command_bytes as u64),
        wire.commands_per_block,
        wire.transaction_weight,
    );
    let _ = writeln!(
        out,
        "  command-count ceiling        {} ({} commands — only reachable by near-weightless commands)",
        human_bytes(wire.max_block_command_bytes as u64),
        wire.max_commands_in_block,
    );

    let _ = writeln!(out, "\nNETWORK REQUIREMENTS (derived; not graded — same on every host)");
    let _ = writeln!(out, "{}", "-".repeat(78));
    let _ = writeln!(
        out,
        "  {:<24} {:>8} {:>10} {:>11} {:>11}",
        "scenario", "interval", "follow", "propose", "disk/epoch"
    );
    for sc in &cap.scenarios {
        let _ = writeln!(
            out,
            "  {:<24} {:>7.2}s {:>7.1} Mbps {:>6.1} Mbps {:>11}",
            sc.name,
            sc.block_interval_secs,
            sc.follow_mbps,
            sc.propose_burst_mbps,
            human_bytes(sc.history_ceiling_bytes),
        );
        for line in wrap(&sc.basis, 68) {
            let _ = writeln!(out, "    {line}");
        }
    }
    let _ = writeln!(
        out,
        "\n  follow   = sustained; forwarding to the mesh, receiving and votes. NOT self-paced — the"
    );
    let _ = writeln!(
        out,
        "             rest of the committee sets this rate and a node must keep up or miss votes."
    );
    let _ = writeln!(
        out,
        "  propose  = burst to push one block to the mesh within {:.0}s, paid only when leading.",
        cap.propose_target_secs
    );
    let _ = writeln!(
        out,
        "             Self-paced: a slow link makes slower blocks, not missed proposals, until the"
    );
    let _ = writeln!(out, "             leader timeout.");
    let _ = writeln!(
        out,
        "\n  transaction gossip           {:.3} Mbps per sustained TPS (independent of block rate)",
        cap.gossip_mbps_per_tps
    );
    if let Some(worst) = cap
        .scenarios
        .iter()
        .max_by(|a, b| a.follow_mbps.total_cmp(&b.follow_mbps))
    {
        let _ = writeln!(
            out,
            "  gossip parity                {:.0} TPS — where gossip equals the {} follow cost",
            worst.follow_mbps / cap.gossip_mbps_per_tps,
            worst.name,
        );
    }
    let _ = writeln!(
        out,
        "  disk figures assume every block is full, and prune beyond epoch_history_length={}",
        cap.epoch_history_length
    );
    let _ = writeln!(
        out,
        "  live substate growth         NOT MEASURED — the unbounded term; needs bytes-per-substate"
    );
    let _ = writeln!(
        out,
        "                               from a running network with representative traffic"
    );
}

fn render_findings(out: &mut String, report: &Report) {
    let _ = writeln!(out, "\nVERDICT: {}", report.grade.label());
    let _ = writeln!(out, "{}", "=".repeat(78));
    for finding in &report.findings {
        let _ = writeln!(
            out,
            "[{}] {:<18} {}",
            finding.grade.label(),
            finding.axis,
            finding.headline
        );
        for line in wrap(&finding.detail, 74) {
            let _ = writeln!(out, "       {line}");
        }
    }
}

/// Greedy word wrap. The report is read in a terminal, and unwrapped detail lines are the fastest
/// way to make a verdict unreadable.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if !current.is_empty() && current.len() + 1 + word.len() > width {
            lines.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

/// Prints the headline metrics of two runs side by side. Used to check a candidate machine against
/// a known-good one, or the same machine before and after a configuration change.
pub fn render_comparison(baseline: &Report, current: &Report) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "\nCOMPARISON: {} (baseline) vs {} (this run)",
        baseline.host.label, current.host.label
    );
    let _ = writeln!(out, "{}", "=".repeat(78));
    let _ = writeln!(
        out,
        "{:<32} {:>14} {:>14} {:>10}",
        "metric", "baseline", "current", "change"
    );

    let mut row = |name: &str, base: Option<f64>, cur: Option<f64>, unit: &str| {
        let (Some(base), Some(cur)) = (base, cur) else {
            return;
        };
        let change = if base == 0.0 {
            f64::NAN
        } else {
            (cur / base - 1.0) * 100.0
        };
        let _ = writeln!(
            out,
            "{:<32} {:>13.2}{} {:>13.2}{} {:>9.1}%",
            name, base, unit, cur, unit, change
        );
    };

    row(
        "weight/s",
        baseline.execution.as_ref().map(|e| e.block.weight_per_sec),
        current.execution.as_ref().map(|e| e.block.weight_per_sec),
        "",
    );
    row(
        "WASM points/ms (millions)",
        baseline.execution.as_ref().map(|e| e.wasm_rate_points_per_ms / 1e6),
        current.execution.as_ref().map(|e| e.wasm_rate_points_per_ms / 1e6),
        "",
    );
    row(
        "worst-case block (s)",
        baseline.projections.as_ref().map(|p| p.validation_block_secs),
        current.projections.as_ref().map(|p| p.validation_block_secs),
        "",
    );
    row(
        "signature verifies/s",
        baseline.native.as_ref().map(|n| n.signature_verifies_per_sec),
        current.native.as_ref().map(|n| n.signature_verifies_per_sec),
        "",
    );
    row(
        "fsync p99 (ms)",
        baseline.storage.as_ref().map(|s| s.fsync.p99_ms),
        current.storage.as_ref().map(|s| s.fsync.p99_ms),
        "",
    );
    row(
        "sequential write (MiB/s)",
        baseline.storage.as_ref().map(|s| s.sequential_write_mib_per_sec),
        current.storage.as_ref().map(|s| s.sequential_write_mib_per_sec),
        "",
    );

    out
}
