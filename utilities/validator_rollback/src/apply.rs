//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    iter,
    ops::Deref,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, anyhow, bail};
use clap::Args as ClapArgs;
use tari_ootle_common_types::{Epoch, ShardGroup, optional::Optional, shard::Shard};
use tari_ootle_p2p::PeerAddress;
use tari_ootle_storage::{
    StateStore,
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    consensus_models::{EpochCheckpoint, RollbackHistoryEntry},
};
use tari_state_store_rocksdb::{
    DatabaseOptions,
    RocksDbStateStore,
    codecs::ByteColumn,
    column_families::bookkeeping::{CURRENT_SCHEMA_VERSION, DatabaseMigrationVersion},
};
use tari_state_tree::Version;

use crate::{
    audit::AuditWriter,
    audit_writer::{AuditCounters, write_block_plan, write_footer, write_header, write_substate_plan},
    storage::{
        rollback_delete_after_epoch,
        rollback_history_insert,
        rollback_plan_collect_blocks,
        rollback_plan_collect_substates,
    },
};

#[derive(Debug, ClapArgs)]
pub struct Args {
    /// Path to the validator's RocksDB state-db directory. Must be writable and not
    /// held by a running validator (RocksDB's LOCK file enforces this).
    #[clap(long)]
    state_db: PathBuf,

    /// Epoch to roll back to. The validator must have a local `EpochCheckpoint` at this
    /// epoch — you can only roll back to a checkpoint the node already holds.
    #[clap(long)]
    target_epoch: u64,

    /// Narrow down the checkpoint lookup when a validator participated in multiple shard
    /// groups at `target_epoch`. Format: `start:end_inclusive`, e.g. `0:63`.
    #[clap(long, parse(try_from_str = parse_shard_group))]
    shard_group: Option<ShardGroup>,

    /// Where to write the audit file. Defaults to
    /// `./rollback-audit-<target_epoch>-<unix_secs>.bin` in the current working directory.
    #[clap(long)]
    audit_out: Option<PathBuf>,

    /// Generate the audit file without mutating the state store.
    #[clap(long)]
    dry_run: bool,
}

fn parse_shard_group(s: &str) -> Result<ShardGroup, String> {
    let (start_s, end_s) = s
        .split_once(':')
        .ok_or_else(|| format!("expected `start:end_inclusive`, got {s:?}"))?;
    let start: u32 = start_s.parse().map_err(|_| "invalid shard_group start".to_string())?;
    let end: u32 = end_s.parse().map_err(|_| "invalid shard_group end".to_string())?;
    ShardGroup::new_checked(Shard::from_u32(start), Shard::from_u32(end))
        .ok_or_else(|| format!("invalid shard_group: {start}:{end}"))
}

pub fn run(args: Args) -> anyhow::Result<()> {
    run_with_options(ApplyOptions {
        state_db: args.state_db,
        target_epoch: args.target_epoch,
        shard_group: args.shard_group,
        audit_out: args.audit_out,
        dry_run: args.dry_run,
    })
    .map(|_| ())
}

/// Structured counterpart to [`Args`] for non-CLI callers (integration tests, runbook
/// automation). Prefer this over synthesising [`Args`] by hand.
#[derive(Debug, Clone)]
pub struct ApplyOptions {
    pub state_db: PathBuf,
    pub target_epoch: u64,
    pub shard_group: Option<ShardGroup>,
    pub audit_out: Option<PathBuf>,
    pub dry_run: bool,
}

/// Outcome returned by [`run_with_options`], suitable for assertions in tests.
#[derive(Debug, Clone)]
pub struct ApplyOutcome {
    pub audit_path: PathBuf,
    pub target_epoch: Epoch,
    pub shard_group: ShardGroup,
    pub substates_removed: u64,
    pub substates_rewound: u64,
    pub substate_transitions: u64,
    pub transactions_unfinalised: u64,
    pub blocks_deleted: u64,
    pub dry_run: bool,
}

/// Refuses to operate on a database whose schema version is not the one this build encodes.
///
/// The rollback primitives write through the current key encodings. Against an older database those encodings do not
/// match what is on disk, so a cascade silently misses the rows a pending migration has yet to rewrite - leaving
/// orphans that the migration then promotes into live entries referring to rolled-back blocks. Against a newer
/// database the encodings are simply unknown to this build.
fn check_schema_version(store: &RocksDbStateStore<PeerAddress>) -> anyhow::Result<()> {
    let version = store.with_read_tx(|tx| -> anyhow::Result<_> {
        let version = tx
            .db()
            .cf(DatabaseMigrationVersion)?
            .get(&ByteColumn, "check_schema_version")
            .optional()?;
        Ok(version)
    })?;

    let Some(version) = version else {
        bail!(
            "State db has no schema version recorded, so it has never been bootstrapped by a validator. There is \
             nothing to roll back."
        );
    };
    match version.cmp(&CURRENT_SCHEMA_VERSION) {
        std::cmp::Ordering::Equal => Ok(()),
        std::cmp::Ordering::Less => bail!(
            "State db is at schema version {version} but this tool writes version {CURRENT_SCHEMA_VERSION}. Start the \
             validator once to migrate the database, stop it again, then re-run the rollback."
        ),
        std::cmp::Ordering::Greater => bail!(
            "State db is at schema version {version}, which is newer than the {CURRENT_SCHEMA_VERSION} this tool \
             understands. Use a rollback tool built from the same release as the validator."
        ),
    }
}

/// Run the rollback flow with explicit options. Returns a summary of what the audit
/// footer recorded so callers can assert on impact.
pub fn run_with_options(opts: ApplyOptions) -> anyhow::Result<ApplyOutcome> {
    let target_epoch = Epoch(opts.target_epoch);
    let now_unix = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let audit_path = opts
        .audit_out
        .clone()
        .unwrap_or_else(|| PathBuf::from(format!("./rollback-audit-{}-{}.bin", opts.target_epoch, now_unix)));

    // Open exclusive regardless of dry-run. The tool must refuse to run against a live
    // validator either way — a dry-run against a process that then commits a block
    // after we snapshot would produce a stale audit.
    let store: RocksDbStateStore<PeerAddress> = RocksDbStateStore::open(&opts.state_db, DatabaseOptions::default())
        .with_context(|| format!("opening state db at {:?}", opts.state_db))?;

    check_schema_version(&store)?;

    // Resolve the checkpoint for target_epoch.
    let (checkpoint, shard_group) = locate_checkpoint(&store, target_epoch, opts.shard_group)?;
    let pre_rollback_tip = tip_epoch_and_block(&store)?;

    let state_versions: Vec<(Shard, u64)> = iter::once(Shard::global())
        .chain(shard_group.shard_iter())
        .map(|shard| (shard, checkpoint.get_shard_state_version(shard)))
        .collect();

    // Read-only audit generation.
    let substate_rows = store.with_read_tx(|tx| -> anyhow::Result<_> {
        let mut rows = Vec::new();
        for (shard, version) in &state_versions {
            let mut per_shard = rollback_plan_collect_substates(tx, *shard, *version)?;
            rows.append(&mut per_shard);
        }
        Ok(rows)
    })?;
    let block_rows = store.with_read_tx(|tx| rollback_plan_collect_blocks(tx, target_epoch))?;

    let audit_file =
        std::fs::File::create(&audit_path).with_context(|| format!("creating audit file at {:?}", audit_path))?;
    let mut writer = AuditWriter::new(std::io::BufWriter::new(audit_file))?;

    write_header(
        &mut writer,
        target_epoch,
        shard_group,
        pre_rollback_tip,
        state_versions.clone(),
        env!("CARGO_PKG_VERSION"),
        now_unix,
        opts.dry_run,
    )?;

    let mut counters = AuditCounters::default();
    write_substate_plan(&mut writer, &mut counters, &substate_rows)?;
    write_block_plan(&mut writer, &mut counters, &block_rows)?;
    let footer_snapshot = clone_counters(&counters);
    write_footer(&mut writer, counters)?;
    writer.finish()?;

    println!("Audit file written: {}", audit_path.display());
    println!(
        "  substates_removed:        {}\n  substates_rewound:        {}\n  substate_transitions:     {}\n  \
         transactions_unfinalised: {}\n  blocks_deleted:           {}",
        footer_snapshot.substates_removed,
        footer_snapshot.substates_rewound,
        footer_snapshot.substate_transitions,
        footer_snapshot.transactions_unfinalised,
        footer_snapshot.blocks_deleted,
    );

    let outcome = ApplyOutcome {
        audit_path: audit_path.clone(),
        target_epoch,
        shard_group,
        substates_removed: footer_snapshot.substates_removed,
        substates_rewound: footer_snapshot.substates_rewound,
        substate_transitions: footer_snapshot.substate_transitions,
        transactions_unfinalised: footer_snapshot.transactions_unfinalised,
        blocks_deleted: footer_snapshot.blocks_deleted,
        dry_run: opts.dry_run,
    };

    if opts.dry_run {
        println!("Dry-run: state store not modified.");
        return Ok(outcome);
    }

    // Apply the rollback in a single atomic write transaction.
    let audit_basename = audit_path
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or("")
        .to_string();
    store.with_write_tx(|tx| -> anyhow::Result<()> {
        for (shard, version) in &state_versions {
            rewind_shard(tx, *shard, *version)?;
        }
        rollback_delete_after_epoch(tx, target_epoch).context("rollback_delete_after_epoch")?;
        rollback_history_insert(tx, &RollbackHistoryEntry {
            target_epoch,
            shard_group,
            applied_at_unix_secs: now_unix,
            tool_version: env!("CARGO_PKG_VERSION").to_string(),
            audit_file_basename: audit_basename,
        })
        .context("rollback_history_insert")?;
        Ok(())
    })?;

    println!("Rollback applied successfully.");
    Ok(outcome)
}

/// Rewinds `shard`'s state tree and substates to `version`.
///
/// A rewind across a destruction whose value epoch GC has pruned restores the substate without its value. Consensus
/// cannot execute against it and state sync never reports the node up to date, so the rollback is refused.
fn rewind_shard<TTx>(tx: &mut TTx, shard: Shard, version: Version) -> anyhow::Result<()>
where
    TTx: StateStoreWriteTransaction + Deref,
    TTx::Target: StateStoreReadTransaction,
{
    tx.state_tree_truncate_to_version(shard, version)
        .with_context(|| format!("state_tree_truncate_to_version(shard={shard}, version={version})"))?;
    tx.substates_rewind_to_state_version(shard, version)
        .with_context(|| format!("substates_rewind_to_state_version(shard={shard}, version={version})"))?;
    if let Some((address, created_at)) = tx
        .state_sync_missing_values_get_earliest(shard, 0)
        .with_context(|| format!("state_sync_missing_values_get_earliest(shard={shard})"))?
    {
        bail!(
            "rolling {shard} back to v{version} leaves the substate at {address} (created at v{created_at}) up \
             without its value, which epoch GC has pruned; roll back to a later epoch"
        );
    }
    Ok(())
}

fn locate_checkpoint(
    store: &RocksDbStateStore<PeerAddress>,
    target_epoch: Epoch,
    requested_shard_group: Option<ShardGroup>,
) -> anyhow::Result<(EpochCheckpoint, ShardGroup)> {
    if let Some(sg) = requested_shard_group {
        let ck = store.with_read_tx(|tx| tx.epoch_checkpoint_get_by_shard_group(target_epoch, sg))?;
        return Ok((ck, sg));
    }
    // Caller didn't specify. Enumerate up to a small cap and filter to the target epoch.
    // A single validator's DB should hold at most 1–2 checkpoints per epoch (one per shard
    // group it participated in); 32 is a generous upper bound.
    let candidates = store.with_read_tx(|tx| tx.epoch_checkpoint_get_all_from_epoch(target_epoch, 32))?;
    let at_target: Vec<EpochCheckpoint> = candidates.into_iter().filter(|c| c.epoch() == target_epoch).collect();
    match at_target.as_slice() {
        [] => Err(anyhow!(
            "no local checkpoint at epoch {} — cannot roll back to an epoch this validator did not participate in",
            target_epoch.0
        )),
        [only] => {
            let sg = only
                .checked_shard_group()
                .map_err(|e| anyhow!("checkpoint at epoch {} has an invalid shard group: {e}", target_epoch.0))?;
            Ok((only.clone(), sg))
        },
        many => {
            let groups: Vec<String> = many
                .iter()
                .filter_map(|c| c.checked_shard_group().ok())
                .map(|sg| format!("{}:{}", sg.start().as_u32(), sg.end().as_u32()))
                .collect();
            bail!(
                "multiple checkpoints at epoch {} found for shard groups [{}]; pass --shard-group START:END to \
                 disambiguate",
                target_epoch.0,
                groups.join(", ")
            )
        },
    }
}

fn tip_epoch_and_block(
    store: &RocksDbStateStore<PeerAddress>,
) -> anyhow::Result<Option<(Epoch, tari_consensus_types::BlockId)>> {
    // Best-effort: the last stored epoch checkpoint gives us the most recent complete
    // epoch. Block-id fidelity on the tip isn't important for the audit — the trail is
    // what matters — so we use the checkpoint's computed block id.
    let Some(last) = store.with_read_tx(|tx| tx.epoch_checkpoint_get_last()).ok() else {
        return Ok(None);
    };
    let header = last.header();
    let block_id = tari_consensus_types::BlockId::from(header.calculate_block_id());
    Ok(Some((Epoch(header.epoch), block_id)))
}

fn clone_counters(c: &AuditCounters) -> AuditCounters {
    AuditCounters {
        substates_removed: c.substates_removed,
        substates_rewound: c.substates_rewound,
        substate_transitions: c.substate_transitions,
        transactions_unfinalised: c.transactions_unfinalised,
        blocks_deleted: c.blocks_deleted,
    }
}
