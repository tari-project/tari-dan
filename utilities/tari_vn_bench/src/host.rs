//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Facts about the machine under test, recorded alongside every measurement.
//!
//! A throughput number is only interpretable next to the machine that produced it, and a run is
//! only comparable to another run whose host is described the same way — so this is captured
//! unconditionally, including in `--json`, and is what `--compare` keys its diff against.
//!
//! Everything here is best-effort: the fields come from `/proc`, and a host that does not provide
//! them reports `None` rather than failing the run. A missing CPU model never invalidates a timing.

use std::{fs, thread};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Host {
    /// Operator-supplied name for this machine, so a comparison of two reports says which is which.
    pub label: String,
    pub os: String,
    pub arch: String,
    pub cpu_model: Option<String>,
    /// Schedulable threads. Consensus executes a block's transactions serially, so this is the
    /// wrong number to size against — it is recorded to explain the single-core figures, not to
    /// substitute for them.
    pub logical_cpus: usize,
    /// Physical cores, where `/proc/cpuinfo` reports them. Below `logical_cpus` means SMT is on,
    /// and SMT siblings do not add single-thread speed.
    pub physical_cpus: Option<usize>,
    pub cpu_max_mhz: Option<f64>,
    pub mem_total_bytes: Option<u64>,
    pub mem_available_bytes: Option<u64>,
    /// Peak resident set of the benchmark process itself. Not the node's working set — the
    /// benchmark holds one engine and no consensus, storage or networking state — but a floor
    /// under it, and a check that the engine's own footprint is what it should be.
    pub peak_rss_bytes: Option<u64>,
    /// A debug build's engine runs several times slower than a release build's, which would make
    /// every projection in the report meaningless. Recorded so a report produced by the wrong
    /// binary is identifiable after the fact, and warned about at run time.
    pub debug_assertions: bool,
}

impl Host {
    pub fn detect(label: String) -> Self {
        let cpuinfo = fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
        Self {
            label,
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            cpu_model: field(&cpuinfo, "model name"),
            logical_cpus: thread::available_parallelism().map(|n| n.get()).unwrap_or(1),
            physical_cpus: field(&cpuinfo, "cpu cores").and_then(|v| v.parse().ok()),
            cpu_max_mhz: read_max_mhz(),
            mem_total_bytes: meminfo_bytes("MemTotal"),
            mem_available_bytes: meminfo_bytes("MemAvailable"),
            peak_rss_bytes: None,
            debug_assertions: cfg!(debug_assertions),
        }
    }

    /// Samples the high-water mark. Called at the end of the run, once every phase has allocated
    /// whatever it is going to.
    pub fn record_peak_rss(&mut self) {
        self.peak_rss_bytes = status_field_bytes("VmHWM");
    }

    /// True when SMT is enabled, i.e. the logical count overstates how many transactions-worth of
    /// independent execution the machine really has.
    pub fn smt_enabled(&self) -> bool {
        self.physical_cpus.is_some_and(|physical| physical < self.logical_cpus)
    }
}

/// First value of a `key : value` line in `/proc/cpuinfo`. Every core repeats the same model, so
/// the first is representative on the homogeneous hosts this runs on.
fn field(cpuinfo: &str, key: &str) -> Option<String> {
    cpuinfo
        .lines()
        .find(|line| line.starts_with(key))
        .and_then(|line| line.split_once(':'))
        .map(|(_, value)| value.trim().to_string())
}

/// Maximum non-turbo frequency from cpufreq, in MHz. Absent on hosts without the driver, and on
/// most VMs — which is itself informative, since a guest usually cannot see or pin its own clock.
fn read_max_mhz() -> Option<f64> {
    let khz = fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/cpuinfo_max_freq").ok()?;
    khz.trim().parse::<f64>().ok().map(|khz| khz / 1000.0)
}

fn meminfo_bytes(key: &str) -> Option<u64> {
    let meminfo = fs::read_to_string("/proc/meminfo").ok()?;
    kib_line(&meminfo, key)
}

fn status_field_bytes(key: &str) -> Option<u64> {
    let status = fs::read_to_string("/proc/self/status").ok()?;
    kib_line(&status, key)
}

/// Parses a `Key:   1234 kB` line, the shared shape of `/proc/meminfo` and `/proc/self/status`.
fn kib_line(text: &str, key: &str) -> Option<u64> {
    text.lines()
        .find(|line| line.starts_with(key) && line[key.len()..].starts_with(':'))
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|kib| kib.parse::<u64>().ok())
        .map(|kib| kib * 1024)
}
