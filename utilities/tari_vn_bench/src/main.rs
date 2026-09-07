//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Measures whether a machine can keep up with validator-node consensus, and reports the spec that
//! implies.
//!
//! The tool is a single self-contained binary: the templates it executes are compiled into the
//! `tari_template_builtin` crate, so nothing needs a Rust toolchain, a checkout or a network
//! connection on the machine under test. Copy it to the candidate host and run it.
//!
//! ```text
//! tari-vn-bench --label mainnet-candidate-1 --data-dir /var/lib/tari
//! tari-vn-bench --json --label candidate-1 > candidate-1.json
//! tari-vn-bench --compare candidate-1.json --label candidate-2
//! ```
//!
//! Run it on an idle machine. Every figure is a minimum-of-N, so competing load can only make the
//! machine look worse than it is — but a report taken under load says nothing useful about the
//! machine's ceiling. The report flags a noisy run rather than silently grading one.

mod capacity;
mod execution;
mod host;
mod memory;
mod native;
mod report;
mod stats;
mod storage;
mod wire;

use std::{fs, path::PathBuf};

use clap::Parser;
use tari_consensus::consensus_constants::ConsensusConstants;

use crate::{host::Host, report::Budgets};

#[derive(Parser, Debug)]
#[allow(clippy::struct_excessive_bools)]
#[clap(author, version, about = "Validator node hardware sizing benchmark", long_about = None)]
struct Cli {
    /// Name for this machine, carried into the report so two runs can be told apart.
    #[clap(long, default_value = "unnamed-host")]
    label: String,
    /// Network whose consensus constants the results are graded against.
    #[clap(long, default_value = "mainnet", possible_values = ["mainnet", "testnet", "esmeralda", "devnet"])]
    network: String,
    /// Directory to run the storage benchmark in. This must be on the volume the node's data
    /// directory will live on: a fast root disk says nothing about a slow attached volume.
    #[clap(long, default_value = ".")]
    data_dir: PathBuf,
    /// Read a running validator node's memory footprint and check it against the derived ceiling.
    #[clap(long)]
    vn_pid: Option<u32>,
    /// Epoch length in minutes, used for the per-epoch disk figures. Follows the layer-one epoch
    /// length (10 L1 blocks at a 2 minute L1 block time), so it cannot be derived offline.
    #[clap(long, default_value = "20")]
    epoch_minutes: f64,
    /// Round trip to collect votes from the committee, in milliseconds. Added to this host's
    /// measured block execution time to estimate the interval a saturated network runs at, since
    /// under load the next block is proposed as soon as the previous quorum certificate forms.
    #[clap(long, default_value = "250")]
    committee_rtt_ms: f64,
    /// Seconds this validator is willing to take to get its proposal to the mesh when it leads.
    /// Proposing is self-paced up to the leader timeout, so this is a throughput target rather than
    /// a liveness bound — set it low to avoid being the committee's slowest leader.
    #[clap(long, default_value = "5")]
    propose_target_secs: f64,
    /// Epochs of block history retained before pruning. Matches `DatabaseOptions::epoch_history_length`.
    #[clap(long, default_value = "1")]
    epoch_history_length: u64,
    /// Emit the report as JSON on stdout. Progress and harness output stay on stderr, so this
    /// redirects cleanly.
    #[clap(long)]
    json: bool,
    /// A JSON report from a previous run to compare this one against.
    #[clap(long)]
    compare: Option<PathBuf>,
    /// Fewer samples. Useful for a first look; the margins it reports are wider than they appear.
    #[clap(long)]
    quick: bool,
    #[clap(long)]
    skip_execution: bool,
    #[clap(long)]
    skip_native: bool,
    #[clap(long)]
    skip_storage: bool,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let constants = match cli.network.as_str() {
        "mainnet" => ConsensusConstants::mainnet(),
        "testnet" => ConsensusConstants::testnet(),
        "esmeralda" => ConsensusConstants::esmeralda(),
        "devnet" => ConsensusConstants::DEVNET,
        other => anyhow::bail!("unknown network {other}"),
    };
    let budgets = Budgets::new(&cli.network, &constants);

    let mut host = Host::detect(cli.label.clone());
    if host.debug_assertions {
        eprintln!(
            "WARNING: this is a debug build. The engine runs several times slower than the release build a validator \
             actually uses, so the execution results will be meaningless. Rebuild with --release."
        );
    }

    // The storage phase runs last but is checked first: it is the only phase that can fail on the
    // machine's configuration rather than its speed, and discovering that after several minutes of
    // execution benchmarking costs the whole run.
    if !cli.skip_storage {
        storage::preflight(&cli.data_dir)?;
    }

    let execution = if cli.skip_execution {
        None
    } else {
        eprintln!("[1/3] Executing transactions through the engine...");
        Some(execution::measure(constants.max_block_validation_weight, cli.quick)?)
    };

    let native = if cli.skip_native {
        None
    } else {
        eprintln!("[2/3] Timing native verification...");
        Some(native::measure(cli.quick)?)
    };

    // A failure here is reported rather than propagated: the execution and native phases have
    // already run by this point, and their results are worth more than the storage figures are
    // worth aborting for.
    let (storage, storage_error) = if cli.skip_storage {
        (None, None)
    } else {
        eprintln!("[3/3] Measuring storage at {}...", cli.data_dir.display());
        match storage::measure(&cli.data_dir, cli.quick) {
            Ok(measurement) => (Some(measurement), None),
            Err(e) => {
                eprintln!("WARNING: the storage phase failed: {e:#}");
                (None, Some(format!("{e:#}")))
            },
        }
    };

    // Deterministic and cheap: no host state is involved, so this runs whatever else was skipped.
    let wire = wire::measure(constants.max_commands_in_block, constants.max_block_weight)?;
    // Under load the block rate is set by how fast consensus can cycle, not by the pacemaker, so
    // the saturated projection is anchored to this host's measured execution time. Absent an
    // execution run there is nothing to anchor it to and only the quiet ceiling is reported.
    let saturation_interval_secs = execution
        .as_ref()
        .map(|e| budgets.max_block_validation_weight as f64 / e.block.weight_per_sec + cli.committee_rtt_ms / 1000.0);
    let capacity = capacity::project(
        &wire,
        &budgets,
        cli.epoch_minutes * 60.0,
        cli.epoch_history_length,
        saturation_interval_secs,
        cli.propose_target_secs,
    );

    let memory = memory::budget(cli.vn_pid);
    host.record_peak_rss();

    let report = report::Report::build(
        host,
        budgets,
        execution,
        native,
        storage,
        storage_error,
        memory,
        wire,
        capacity,
    );

    if cli.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", report::render(&report));
    }

    if let Some(baseline_path) = &cli.compare {
        let baseline: report::Report = serde_json::from_str(&fs::read_to_string(baseline_path)?)?;
        eprint!("{}", report::render_comparison(&baseline, &report));
    }

    Ok(())
}
