//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Durability and write throughput of the volume the node's data directory will live on.
//!
//! Two numbers decide whether storage is a problem, and both must be measured on the real volume —
//! not on `/tmp`, and not on the machine's boot disk if the data directory is elsewhere.
//!
//! * **fsync latency** is the floor under every RocksDB commit. It cannot be faked by the page cache: a sync only
//!   returns once the device says the data is durable. This is what separates an NVMe from a SATA SSD from a throttled
//!   network volume, and it is the number that decides whether committing a block fits inside a view.
//! * **Sequential write throughput** is what state sync and compaction run at. It decides how long a node takes to join
//!   a shard group or to recover, which is an availability property even though it is not a per-view one.
//!
//! Random-read IOPS is deliberately not measured here: without `O_DIRECT` or the ability to drop
//! the page cache — neither of which a benchmark can assume it may do — any figure produced would
//! be the page cache's, not the device's. Measure it with `fio` if the fsync result is marginal.

use std::{
    fs::{self, File, OpenOptions},
    io::{Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    time::Instant,
};

use serde::{Deserialize, Serialize};

use crate::stats::Sample;

/// Payload per sync. Small on purpose: this measures the round trip to durability, not bandwidth,
/// and a large write would blur the two.
const SYNC_PAYLOAD_BYTES: usize = 4096;
const SYNC_TRIALS: usize = 300;
const SYNC_TRIALS_QUICK: usize = 60;

/// Size of the sequential write test. Comfortably past any plausible write-back cache, so the
/// figure reported is the device's sustained rate rather than the rate at which it accepts data
/// into RAM.
const SEQ_WRITE_BYTES: u64 = 512 * 1024 * 1024;
const SEQ_WRITE_BYTES_QUICK: u64 = 128 * 1024 * 1024;
const SEQ_CHUNK_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageMeasurement {
    pub path: String,
    /// Filesystem backing `path`, when `/proc/mounts` identifies it. A `tmpfs` here invalidates the
    /// whole phase: it measures RAM, and a node's data directory must not live there.
    pub filesystem: Option<String>,
    /// Round trip to durability for a small write.
    pub fsync: Sample,
    pub sequential_write_mib_per_sec: f64,
    pub bytes_written: u64,
}

pub fn measure(data_dir: &Path, quick: bool) -> anyhow::Result<StorageMeasurement> {
    fs::create_dir_all(data_dir)?;
    let scratch = data_dir.join(".tari-vn-bench.tmp");

    let filesystem = filesystem_for(data_dir);
    let fsync = measure_fsync(&scratch, if quick { SYNC_TRIALS_QUICK } else { SYNC_TRIALS })?;
    let target_bytes = if quick { SEQ_WRITE_BYTES_QUICK } else { SEQ_WRITE_BYTES };
    let sequential_write_mib_per_sec = measure_sequential_write(&scratch, target_bytes)?;

    // Best effort: a leftover scratch file is untidy but not a failure worth aborting the report
    // for, and the caller may have no way to act on it anyway.
    let _ignore = fs::remove_file(&scratch);

    Ok(StorageMeasurement {
        path: data_dir.display().to_string(),
        filesystem,
        fsync,
        sequential_write_mib_per_sec,
        bytes_written: target_bytes,
    })
}

/// Times `write` + `sync_data` pairs, which is what RocksDB does to its write-ahead log on commit.
///
/// The write rotates across a pre-allocated region rather than appending, so the cost measured is
/// the sync itself and not the filesystem extending a file.
fn measure_fsync(path: &Path, trials: usize) -> anyhow::Result<Sample> {
    let mut file = OpenOptions::new().create(true).write(true).truncate(true).open(path)?;
    let payload = vec![0xA5u8; SYNC_PAYLOAD_BYTES];
    let region_blocks = 256u64;
    file.set_len(region_blocks * SYNC_PAYLOAD_BYTES as u64)?;
    file.sync_all()?;

    // Warm up: the first sync on a fresh file pays metadata work that no later one does.
    file.write_all(&payload)?;
    file.sync_data()?;

    let mut ms = Vec::with_capacity(trials);
    for i in 0..trials {
        file.seek(SeekFrom::Start((i as u64 % region_blocks) * SYNC_PAYLOAD_BYTES as u64))?;
        let started = Instant::now();
        file.write_all(&payload)?;
        file.sync_data()?;
        ms.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    Ok(Sample::from_millis(ms))
}

/// Writes `target_bytes` sequentially and syncs at the end, reporting the sustained rate.
///
/// The final sync is inside the timed section on purpose: without it the figure is the rate at
/// which the page cache accepted the data, which on any machine with free RAM is not a disk
/// measurement at all.
fn measure_sequential_write(path: &Path, target_bytes: u64) -> anyhow::Result<f64> {
    let mut file = File::create(path)?;
    let chunk = vec![0x5Au8; SEQ_CHUNK_BYTES];

    let started = Instant::now();
    let mut written = 0u64;
    while written < target_bytes {
        file.write_all(&chunk)?;
        written += SEQ_CHUNK_BYTES as u64;
    }
    file.sync_all()?;
    let secs = started.elapsed().as_secs_f64();

    Ok((written as f64 / (1024.0 * 1024.0)) / secs)
}

/// Filesystem type of the mount that contains `path`, by longest-prefix match over `/proc/mounts`.
fn filesystem_for(path: &Path) -> Option<String> {
    let target = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let mounts = fs::read_to_string("/proc/mounts").ok()?;

    let mut best: Option<(usize, String)> = None;
    for line in mounts.lines() {
        let mut fields = line.split_whitespace();
        let (_device, mount_point, fs_type) = (fields.next()?, fields.next()?, fields.next()?);
        let mount_path = PathBuf::from(mount_point);
        if target.starts_with(&mount_path) {
            let depth = mount_path.components().count();
            if best.as_ref().is_none_or(|(best_depth, _)| depth > *best_depth) {
                best = Some((depth, fs_type.to_string()));
            }
        }
    }
    best.map(|(_, fs_type)| fs_type)
}
