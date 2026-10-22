//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Write-side rollback primitives that operate against the rocksdb state store directly.
//!
//! Consensus never calls these, so they live with the rollback tool and take a
//! `&mut RocksDbStateStoreWriteTransaction`.
//!
//! `rollback_delete_after_epoch` reaches into per-block trait helpers (block diffs,
//! block transaction executions, pending state-tree diffs, substate locks, lock
//! conflicts, transaction-pool state updates) — those helpers stay on
//! `StateStoreWriteTransaction` because consensus uses them as well, so we just call
//! through the trait via `&mut tx`.

use log::*;
use serde::{Serialize, de::DeserializeOwned};
use tari_consensus_types::{BlockId, PcId, TcId};
use tari_ootle_common_types::{Epoch, NodeAddressable, NodeHeight, ShardGroup};
use tari_ootle_storage::{Ordering, StateStoreWriteTransaction, StorageError, consensus_models::RollbackHistoryEntry};
use tari_state_store_rocksdb::{
    codecs::ByteColumn,
    column_families::{
        block,
        block::BlockCf,
        bookkeeping::{
            HighPcCf,
            HighTcCf,
            HighestSeenBlockCf,
            LastExecutedCf,
            LastProposedCf,
            LastSentNewViewCf,
            LastSentVoteCf,
            LastVotedCf,
            LeafBlockCf,
            LockedBlockCf,
        },
        certificates::{proposal::ProposalCertificateCf, timeout::TimeoutCertificateCf},
        chain::{self, CommittedParentChildChainIndex},
        epoch_checkpoint::EpochCheckpointCf,
        finalized_transaction::FinalizedTransactionLinkCf,
        foreign_proposal::{self, EpochIndex as ForeignProposalEpochIndex, ForeignProposalCf},
        rollback_history::RollbackHistoryCf,
        validator_node_epoch_stats::ValidatorNodeEpochStatsCf,
    },
    writer::RocksDbStateStoreWriteTransaction,
};
use tari_template_lib_types::crypto::RistrettoPublicKeyBytes;

use super::types::RollbackDeleteStats;

const LOG_TARGET: &str = "tari::ootle::validator_rollback::storage::write";

/// Append a rollback-history breadcrumb. Called inside the same write transaction as
/// the truncate / rewind / delete-after-epoch ops so either all four succeed or none do.
pub fn rollback_history_insert<'a, TAddr>(
    tx: &mut RocksDbStateStoreWriteTransaction<'a, TAddr>,
    entry: &RollbackHistoryEntry,
) -> Result<(), StorageError>
where
    TAddr: NodeAddressable + Serialize + DeserializeOwned + 'a,
{
    const OPERATION: &str = "rollback_history_insert";
    tx.db()
        .cf(RollbackHistoryCf)?
        .put(&(entry.applied_at_unix_secs, entry.target_epoch), entry, OPERATION)?;
    Ok(())
}

/// Delete every epoch-indexed record with `epoch > target_epoch`, and clear the singleton
/// bookkeeping pointers. Paired with `state_tree_truncate_to_version` and
/// `substates_rewind_to_state_version`, this is the storage side of a break-glass rollback
/// to the `target_epoch` checkpoint.
///
/// Caller invokes this *inside the same write transaction* as the state-tree truncate,
/// substate rewind, and `rollback_history_insert`.
///
/// After commit, consensus resumes via the `create_genesis_block_if_required` path in
/// hotstuff: with no blocks at `(current_epoch, height=0)` the worker creates a fresh
/// genesis and repopulates all bookkeeping pointers from it.
#[allow(clippy::too_many_lines)]
pub fn rollback_delete_after_epoch<'a, TAddr>(
    tx: &mut RocksDbStateStoreWriteTransaction<'a, TAddr>,
    target_epoch: Epoch,
) -> Result<RollbackDeleteStats, StorageError>
where
    TAddr: NodeAddressable + Serialize + DeserializeOwned + 'a,
{
    const OPERATION: &str = "rollback_delete_after_epoch";

    let mut stats = RollbackDeleteStats::default();
    let start_epoch = target_epoch.saturating_add(Epoch(1));

    // 1. Collect block_ids where epoch > target. We collect first so that per-block cascade helpers (which take `&mut
    //    self`) can run without holding an immutable CF-handle borrow over the iteration.
    let to_delete: Vec<(Epoch, NodeHeight, BlockId)> = tx
        .db()
        .cf(block::ByEpochQuery)?
        .query_range_key_iterator(Ordering::Ascending, start_epoch..Epoch::max())
        .collect::<Result<Vec<_>, _>>()?;

    for (epoch, height, block_id) in &to_delete {
        // Load the block before deleting so we can walk its commands and clean up per-tx
        // records that the block-id-keyed cascade below doesn't cover, keyed as they are
        // by transaction id alone.
        let block = tx.db().cf(BlockCf)?.get(block_id, OPERATION)?;

        // Per-block cascades via existing helpers. Keeps this list consistent with what
        // block insertion creates, so the cascade stays correct as new tables are added.
        // These helpers stay on `StateStoreWriteTransaction` because consensus also uses
        // them — we just call through the trait via `&mut tx`.
        tx.block_diffs_remove(block_id)?;
        tx.block_transaction_executions_remove_any_by_block_id(block_id)?;
        tx.pending_state_tree_diffs_remove_by_block(block_id)?;
        tx.substate_locks_remove_any_by_block_id(block_id)?;
        tx.transaction_pool_state_updates_remove_any_by_block_id(block_id)?;
        tx.lock_conflicts_remove_by_block_id(block_id)?;

        let finalized_link_cf = tx.db().cf(FinalizedTransactionLinkCf)?;
        // Finalising commands mark this block as where the transaction's finalize QC
        // committed, so `FinalizedTransactionLinkCf[tx_id]` was written here. Deleting it
        // restores the "not-yet-finalized" state so `finalized_transaction_execution_get`
        // returns NotFound post-rollback.
        for tx_id in block.all_finalising_transactions_ids() {
            finalized_link_cf.delete(tx_id, OPERATION)?;
        }

        // Delete the block record and its chain/epoch indexes. Use the idempotent
        // `delete` throughout — a block is either in both chain indexes or neither (it
        // was committed vs. still pending), so one of `CommittedParentChildChainIndex`
        // / `PendingChainIndex` will always be a miss and the `delete_or_not_found`
        // variant would panic on that miss.
        tx.db().cf(BlockCf)?.delete(block_id, OPERATION)?;
        tx.db()
            .cf(block::EpochHeightIndex)?
            .delete(&(*epoch, *height, *block_id), OPERATION)?;
        tx.db()
            .cf(CommittedParentChildChainIndex)?
            .delete(block_id, OPERATION)?;
        tx.db().cf(chain::PendingChainIndex)?.delete(block_id, OPERATION)?;
        // PendingParentChildIndex is keyed by (parent, child). Precise deletion needs the
        // block's parent, which itself may already be gone. Stale entries are benign —
        // all index consumers start from known block_ids and won't reach orphans.
    }
    stats.blocks_deleted = to_delete.len();

    // 2. Proposal certificates with epoch > target.
    let pc_cf = tx.db().cf(ProposalCertificateCf)?;
    let pc_query = tx
        .db()
        .cf(tari_state_store_rocksdb::column_families::certificates::proposal::ByEpochQuery)?;
    let pc_keys: Vec<(Epoch, PcId)> = pc_query
        .query_range_key_iterator(Ordering::Ascending, start_epoch..Epoch::max())
        .collect::<Result<Vec<_>, _>>()?;
    for key in &pc_keys {
        pc_cf.delete(key, OPERATION)?;
    }
    stats.certificates_deleted += pc_keys.len();

    // 3. Timeout certificates with epoch > target.
    let tc_cf = tx.db().cf(TimeoutCertificateCf)?;
    let tc_query = tx
        .db()
        .cf(tari_state_store_rocksdb::column_families::certificates::timeout::ByEpochQuery)?;
    let tc_keys: Vec<(Epoch, TcId)> = tc_query
        .query_range_key_iterator(Ordering::Ascending, start_epoch..Epoch::max())
        .collect::<Result<Vec<_>, _>>()?;
    for key in &tc_keys {
        tc_cf.delete(key, OPERATION)?;
    }
    stats.certificates_deleted += tc_keys.len();

    // 4. Epoch checkpoints with epoch > target.
    let cp_cf = tx.db().cf(EpochCheckpointCf)?;
    let cp_query = tx
        .db()
        .cf(tari_state_store_rocksdb::column_families::epoch_checkpoint::ByEpochQuery)?;
    let cp_keys: Vec<(Epoch, ShardGroup)> = cp_query
        .query_range_key_iterator(Ordering::Ascending, start_epoch..Epoch::max())
        .collect::<Result<Vec<_>, _>>()?;
    for key in &cp_keys {
        cp_cf.delete(key, OPERATION)?;
    }
    stats.checkpoints_deleted = cp_keys.len();

    // 5. Foreign proposals with epoch > target. Use the ByEpochQuery to find block_ids, then delete via the existing
    //    per-block helper which handles all the secondary indexes.
    let fp_query = tx.db().cf(foreign_proposal::ByEpochQuery)?;
    let fp_epoch_index_cf = tx.db().cf(ForeignProposalEpochIndex)?;
    let fp_epoch_keys: Vec<((Epoch, BlockId), foreign_proposal::ForeignProposalEpochIndexData)> = fp_query
        .query_range_iterator(Ordering::Ascending, start_epoch..Epoch::max())
        .collect::<Result<Vec<_>, _>>()?;
    for ((epoch, block_id), _) in &fp_epoch_keys {
        // Idempotent delete in case a prior cascade already removed it.
        tx.db().cf(ForeignProposalCf)?.delete(block_id, OPERATION)?;
        fp_epoch_index_cf.delete(&(*epoch, *block_id), OPERATION)?;
    }
    stats.foreign_proposals_deleted = fp_epoch_keys.len();

    // 6. Validator epoch stats with epoch > target.
    let stats_cf = tx.db().cf(ValidatorNodeEpochStatsCf)?;
    let stats_query = tx
        .db()
        .cf(tari_state_store_rocksdb::column_families::validator_node_epoch_stats::ByEpochQuery)?;
    let stats_keys: Vec<(Epoch, RistrettoPublicKeyBytes)> = stats_query
        .query_range_key_iterator(Ordering::Ascending, start_epoch..Epoch::max())
        .collect::<Result<Vec<_>, _>>()?;
    for key in &stats_keys {
        stats_cf.delete(key, OPERATION)?;
    }
    stats.validator_stats_deleted = stats_keys.len();

    // 7. Clear bookkeeping singletons. These are all keyed by `ByteColumn`; deleting the single entry removes the
    //    pointer entirely. On consensus resume the genesis path in `create_genesis_block_if_required` repopulates them
    //    for the fresh epoch. Use idempotent `delete` — a freshly-joined node may not have every singleton populated
    //    yet (e.g. `LastSentVote` before it has ever voted) and this step must not fail in that case.
    tx.db().cf(LeafBlockCf)?.delete(&ByteColumn, OPERATION)?;
    tx.db().cf(LockedBlockCf)?.delete(&ByteColumn, OPERATION)?;
    tx.db().cf(HighestSeenBlockCf)?.delete(&ByteColumn, OPERATION)?;
    tx.db().cf(LastExecutedCf)?.delete(&ByteColumn, OPERATION)?;
    tx.db().cf(LastVotedCf)?.delete(&ByteColumn, OPERATION)?;
    tx.db().cf(LastProposedCf)?.delete(&ByteColumn, OPERATION)?;
    tx.db().cf(LastSentVoteCf)?.delete(&ByteColumn, OPERATION)?;
    tx.db().cf(LastSentNewViewCf)?.delete(&ByteColumn, OPERATION)?;
    tx.db().cf(HighPcCf)?.delete(&ByteColumn, OPERATION)?;
    tx.db().cf(HighTcCf)?.delete(&ByteColumn, OPERATION)?;
    stats.bookkeeping_cleared = true;

    info!(
        target: LOG_TARGET,
        "🔄 Rollback delete for epoch > {target_epoch}: {} blocks, {} certificates, \
         {} checkpoints, {} foreign proposals, {} validator-stats rows; bookkeeping cleared",
        stats.blocks_deleted,
        stats.certificates_deleted,
        stats.checkpoints_deleted,
        stats.foreign_proposals_deleted,
        stats.validator_stats_deleted,
    );

    Ok(stats)
}
