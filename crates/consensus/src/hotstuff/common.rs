//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashMap, iter, ops::ControlFlow};

use indexmap::IndexMap;
use log::*;
use tari_common_types::types::FixedHash;
use tari_consensus_types::{BlockId, HighPc, HighTc, LeafBlock, PcId, ProposalCertificate, ShardGroupAccumulatedData};
use tari_engine_types::{ValidatorFeePool, substate::SubstateDiff};
use tari_ootle_common_types::{
    Epoch,
    NodeAddressable,
    NodeHeight,
    NumPreshards,
    ProtocolVersion,
    ShardGroup,
    committee::{Committee, CommitteeInfo},
    derive_fee_pool_address,
    optional::Optional,
    shard::Shard,
    substate_type::SubstateType,
};
use tari_ootle_storage::{
    ShardScopedTreeStoreReader,
    StateStoreReadTransaction,
    consensus_models::{
        Block,
        BlockHeader,
        BookkeepingModel,
        EpochCheckpoint,
        PendingShardStateTreeDiff,
        SubstateChange,
        TransactionRecord,
        TreeRootSummary,
    },
};
use tari_ootle_transaction::Network;
use tari_state_tree::{JellyfishMerkleTree, SPARSE_MERKLE_PLACEHOLDER_HASH, StateTreeError};
use tari_template_lib_types::crypto::RistrettoPublicKeyBytes;

use crate::{
    hotstuff::{
        HotStuffError,
        block_change_set::ProposedBlockChangeSet,
        commit_proofs::generate_end_of_epoch_commit_proof,
        substate_store::{PendingSubstateStore, ShardedStateTree},
    },
    tracing::TraceTimer,
    traits::LeaderStrategy,
};

const LOG_TARGET: &str = "tari::ootle::consensus::hotstuff::common";

/// Calculates the dummy block required to reach the new height and returns the last dummy block (parent for next
/// proposal). Generates dummy blocks from from_height to new_height _exclusive_.
pub fn calculate_last_dummy_block<TAddr: NodeAddressable, TLeaderStrategy: LeaderStrategy<TAddr>>(
    from_height: NodeHeight,
    new_height: NodeHeight,
    network: Network,
    epoch: Epoch,
    shard_group: ShardGroup,
    parent_block_id: BlockId,
    qc: &ProposalCertificate,
    parent_merkle_root: FixedHash,
    leader_strategy: &TLeaderStrategy,
    local_committee: &Committee<TAddr>,
    parent_timestamp: u64,
    parent_accumulated_data: ShardGroupAccumulatedData,
    parent_epoch_hash: FixedHash,
) -> Option<LeafBlock> {
    let mut dummy = None;
    with_dummy_blocks(
        from_height,
        new_height,
        network,
        epoch,
        shard_group,
        parent_block_id,
        qc,
        parent_merkle_root,
        leader_strategy,
        local_committee,
        parent_timestamp,
        parent_accumulated_data,
        parent_epoch_hash,
        |dummy_block| {
            dummy = Some(dummy_block.as_leaf());
            ControlFlow::Continue(())
        },
    );

    dummy
}

pub fn calculate_dummy_blocks<TAddr: NodeAddressable, TLeaderStrategy: LeaderStrategy<TAddr>>(
    from_height: NodeHeight,
    new_height: NodeHeight,
    network: Network,
    epoch: Epoch,
    shard_group: ShardGroup,
    parent_block_id: BlockId,
    qc: &ProposalCertificate,
    expected_parent_block_id: &BlockId,
    parent_merkle_root: FixedHash,
    leader_strategy: &TLeaderStrategy,
    local_committee: &Committee<TAddr>,
    parent_timestamp: u64,
    parent_accumulated_data: ShardGroupAccumulatedData,
    parent_epoch_hash: FixedHash,
) -> Vec<Block> {
    let mut dummies = Vec::with_capacity(new_height.saturating_sub(from_height).as_u64() as usize);
    with_dummy_blocks(
        from_height,
        new_height,
        network,
        epoch,
        shard_group,
        parent_block_id,
        qc,
        parent_merkle_root,
        leader_strategy,
        local_committee,
        parent_timestamp,
        parent_accumulated_data,
        parent_epoch_hash,
        |dummy_block| {
            if dummy_block.id() == expected_parent_block_id {
                dummies.push(dummy_block);
                ControlFlow::Break(())
            } else {
                dummies.push(dummy_block);
                ControlFlow::Continue(())
            }
        },
    );

    dummies
}

/// Calculates the dummy block required to reach the new height
pub fn calculate_dummy_blocks_from_justify<TAddr: NodeAddressable, TLeaderStrategy: LeaderStrategy<TAddr>>(
    candidate_block: &Block,
    justify_block: &Block,
    leader_strategy: &TLeaderStrategy,
    local_committee: &Committee<TAddr>,
) -> Vec<Block> {
    calculate_dummy_blocks(
        justify_block.height(),
        candidate_block.height(),
        candidate_block.network(),
        candidate_block.epoch(),
        candidate_block.shard_group(),
        *justify_block.id(),
        candidate_block.justify(),
        candidate_block.parent(),
        *justify_block.state_merkle_root(),
        leader_strategy,
        local_committee,
        justify_block.timestamp(),
        *justify_block.header().accumulated_data(),
        *justify_block.epoch_hash(),
    )
}

fn with_dummy_blocks<TAddr, TLeaderStrategy, F>(
    mut current_block_height: NodeHeight,
    new_height: NodeHeight,
    network: Network,
    epoch: Epoch,
    shard_group: ShardGroup,
    mut parent_block_id: BlockId,
    qc: &ProposalCertificate,
    parent_merkle_root: FixedHash,
    leader_strategy: &TLeaderStrategy,
    local_committee: &Committee<TAddr>,
    parent_timestamp: u64,
    parent_accumulated_data: ShardGroupAccumulatedData,
    parent_epoch_hash: FixedHash,
    mut callback: F,
) where
    TAddr: NodeAddressable,
    TLeaderStrategy: LeaderStrategy<TAddr>,
    F: FnMut(Block) -> ControlFlow<()>,
{
    if current_block_height >= new_height {
        error!(
            target: LOG_TARGET,
            "BUG: 🍼 no dummy blocks to calculate. current height {} is greater than new height {}",
            current_block_height,
            new_height,
        );
        return;
    }

    debug!(
        target: LOG_TARGET,
        "🍼 calculating dummy blocks in epoch {} from {} to {}",
        epoch,
        current_block_height,
        new_height,
    );

    let justify_id = qc.calculate_id();
    loop {
        current_block_height += NodeHeight(1);
        if current_block_height == new_height {
            break;
        }
        let view_height = current_block_height - NodeHeight(1);
        let (_, leader) = leader_strategy.get_leader(local_committee, view_height);
        let dummy_header = BlockHeader::dummy_block(
            network,
            ProtocolVersion::at(network, epoch),
            parent_block_id,
            *leader,
            current_block_height,
            justify_id,
            epoch,
            shard_group,
            parent_merkle_root,
            parent_timestamp,
            parent_epoch_hash,
            parent_accumulated_data,
        );
        let dummy_block = Block::new(dummy_header, qc.clone(), Default::default(), None);
        debug!(
            target: LOG_TARGET,
            "🍼 new dummy block: {}",
            dummy_block,
        );
        parent_block_id = *dummy_block.id();

        if callback(dummy_block).is_break() {
            break;
        }
    }
}

pub fn calculate_state_merkle_root<'a, TTx: StateStoreReadTransaction, I: IntoIterator<Item = &'a SubstateChange>>(
    tx: &TTx,
    shard_group: ShardGroup,
    pending_tree_diffs: HashMap<Shard, Vec<PendingShardStateTreeDiff>>,
    changes: I,
    network: Network,
    epoch: Epoch,
) -> Result<(FixedHash, IndexMap<Shard, PendingShardStateTreeDiff>), StateTreeError> {
    let mut change_map = IndexMap::new();

    changes.into_iter().for_each(|ch| {
        // Group by shard
        change_map
            .entry(ch.shard())
            .or_insert_with(Vec::new)
            .push(ch.to_tree_change(network, epoch));
    });
    let mut sharded_tree = ShardedStateTree::new(tx).with_pending_diffs(pending_tree_diffs);
    let root_hash = sharded_tree.put_substate_tree_changes(shard_group, change_map)?;

    Ok((
        FixedHash::new(root_hash.into_array()),
        sharded_tree.into_shard_tree_diffs(),
    ))
}

pub(crate) fn generate_epoch_checkpoint<TTx>(
    tx: &TTx,
    eoe_block: &Block,
    commit_qc: &ProposalCertificate,
) -> Result<EpochCheckpoint, HotStuffError>
where
    TTx: StateStoreReadTransaction,
{
    let commit_proof = generate_end_of_epoch_commit_proof(tx, commit_qc, eoe_block)?;
    let shard_group = eoe_block.shard_group();

    // Fetch the state roots of the shards in the shard group
    let mut shard_tree_summary = IndexMap::with_capacity(shard_group.len() + 1);

    // adding global shard first
    if let Some(version) = tx.state_tree_versions_get_latest(Shard::global())? {
        let scoped_store = ShardScopedTreeStoreReader::new(tx, Shard::global());
        let jmt = JellyfishMerkleTree::new(&scoped_store);
        let root_hash = jmt
            .get_root_hash(version)
            .map_err(|e| HotStuffError::StateTreeError(e.into()))?;

        shard_tree_summary.insert(Shard::global(), TreeRootSummary {
            root_hash,
            state_version: version,
        });
    } else {
        shard_tree_summary.insert(Shard::global(), TreeRootSummary {
            root_hash: SPARSE_MERKLE_PLACEHOLDER_HASH,
            state_version: 0,
        });
    }

    for shard in shard_group.shard_iter() {
        let Some(version) = tx.state_tree_versions_get_latest(shard)? else {
            // At v0 there have been no state changes
            shard_tree_summary.insert(shard, TreeRootSummary {
                root_hash: SPARSE_MERKLE_PLACEHOLDER_HASH,
                state_version: 0,
            });
            continue;
        };

        let scoped_store = ShardScopedTreeStoreReader::new(tx, shard);
        let jmt = JellyfishMerkleTree::new(&scoped_store);
        let root_hash = jmt
            .get_root_hash(version)
            .map_err(|e| HotStuffError::StateTreeError(e.into()))?;

        shard_tree_summary.insert(shard, TreeRootSummary {
            root_hash,
            state_version: version,
        });
    }

    let checkpoint = EpochCheckpoint::new(commit_proof, shard_tree_summary);
    Ok(checkpoint)
}

pub(crate) fn filter_diff_for_committee(committee_info: &CommitteeInfo, diff: &SubstateDiff) -> SubstateDiff {
    let mut filtered_diff = SubstateDiff::new();
    filtered_diff
        .extend_up(
            diff.up_iter()
                .filter(|(id, _)| committee_info.includes_substate_id(id))
                .cloned(),
        )
        .extend_down(
            diff.down_iter()
                .filter(|(id, _)| committee_info.includes_substate_id(id))
                .cloned(),
        )
        .set_once_fee_withdrawals(
            diff.validator_fee_withdrawals()
                .iter()
                .filter(|f| committee_info.includes_substate_id(&f.address.into()))
                .cloned()
                .collect(),
        );
    filtered_diff
}

pub fn apply_leader_fee_to_substate_store<TTx: StateStoreReadTransaction>(
    store: &mut PendingSubstateStore<TTx>,
    claim_public_key_bytes: &RistrettoPublicKeyBytes,
    num_preshards: NumPreshards,
    shard: Shard,
    total_leader_fee: u64,
) -> Result<(), HotStuffError> {
    // Basic defensive checks
    if total_leader_fee == 0 {
        // Nothing to do
        return Ok(());
    }

    let fee_substate_id = derive_fee_pool_address(claim_public_key_bytes, num_preshards, shard)
        .map_err(|err| HotStuffError::InvariantError(err.to_string()))?;
    store.update_in_place(
        &fee_substate_id.into(),
        |value_mut| {
            debug!(target: LOG_TARGET, "🪙 Deposit leader fee {total_leader_fee} into fee pool {fee_substate_id}");
            let substate_type = SubstateType::from(&*value_mut);
            let pool = value_mut.as_validator_fee_pool_mut().ok_or_else(|| {
                HotStuffError::InvariantError(format!(
                    "Unexpected substate type {} was found at address {}",
                    substate_type, fee_substate_id
                ))
            })?;
            if !pool.deposit_direct(total_leader_fee) {
                return Err(HotStuffError::InvariantError(format!(
                    "Failed to deposit leader fee {total_leader_fee} into fee pool {fee_substate_id}"
                )));
            }

            Ok(())
        },
        |_| Ok(ValidatorFeePool::new(*claim_public_key_bytes, total_leader_fee).into()),
    )?;

    Ok(())
}

pub(crate) fn get_highest_seen_justified_view<TTx: StateStoreReadTransaction>(
    tx: &TTx,
    epoch: Epoch,
) -> Result<NodeHeight, HotStuffError> {
    let high_pc = HighPc::get(tx, epoch)
        .optional()?
        .map(|high_pc| high_pc.height())
        .unwrap_or_default();
    let high_tc = HighTc::get(tx, epoch)
        .optional()?
        .map(|high_tc| high_tc.height())
        .unwrap_or_default();
    Ok(high_pc.max(high_tc))
}

/// Records the prepare/accept evidence for the commands in `new_leaf_block` and its not-yet-justified ancestors,
/// now that `justify_id` justifies them, and marks any transaction that this completes as ready to propose.
///
/// The updates are written to `change_set`, which only ever applies to the branch its block is on, so every branch
/// that justifies `new_leaf_block` must call this for itself.
#[allow(clippy::too_many_lines)]
pub fn process_newly_justified_block<TTx: StateStoreReadTransaction>(
    tx: &TTx,
    new_leaf_block: &Block,
    justify_id: PcId,
    local_committee_info: &CommitteeInfo,
    change_set: &mut ProposedBlockChangeSet,
) -> Result<Vec<Block>, HotStuffError> {
    let timer = TraceTimer::info(LOG_TARGET, "Process newly justified chain");
    fn process_newly_justified_block_inner<TTx: StateStoreReadTransaction>(
        tx: &TTx,
        block: &Block,
        new_leaf_block: &LeafBlock,
        justify_id: PcId,
        local_committee_info: &CommitteeInfo,
        change_set: &mut ProposedBlockChangeSet,
    ) -> Result<(), HotStuffError> {
        let timer = TraceTimer::info(LOG_TARGET, "Process newly justified block");
        info!(
            target: LOG_TARGET,
            "✅ New leaf block {} is justified. Updating evidence for transactions",
            block,
        );

        let mut num_applicable_commands = 0;
        if block.is_epoch_end() {
            debug!(
                target: LOG_TARGET,
                "✅ New leaf block {} is an epoch end. No commands to process in process_newly_justified_block",
                block,
            );
            return Ok(());
        }
        let local_shard_group = local_committee_info.shard_group();

        for cmd in block.commands() {
            if !cmd.is_local_prepare() && !cmd.is_local_accept() {
                continue;
            }

            num_applicable_commands += 1;

            let atom = cmd.transaction().expect("Command must be a transaction");

            // NOTE: we use new_leaf_block here to ensure that we are always updating the latest evidence for the
            // transaction.
            let Some(mut pool_tx) = change_set
                .get_transaction_pool_record(tx, new_leaf_block, atom.id())
                .optional()?
            else {
                // Finalizing a transaction removes its pool record, and a finalized transaction has no evidence
                // left to update. Any other absence leaves the transaction unable to make progress.
                if TransactionRecord::is_record_finalized(tx, atom.id())? {
                    debug!(
                        target: LOG_TARGET,
                        "Transaction {} in justified block {} is finalized and no longer in the pool",
                        atom.id(),
                        block,
                    );
                } else {
                    warn!(
                        target: LOG_TARGET,
                        "❓️ Transaction {} in justified block {} is not in the pool and is not finalized. Its \
                         evidence cannot be updated.",
                        atom.id(),
                        block,
                    );
                }
                continue;
            };

            if cmd.is_local_prepare() {
                pool_tx
                    .evidence_mut()
                    .add_shard_group(local_shard_group)
                    .set_prepare_qc(justify_id);
                debug!(
                    target: LOG_TARGET,
                    "🔍 Updating evidence for LocalPrepare command in block {} for transaction {}. {}",
                    block,
                    atom.id(),
                    pool_tx.evidence()
                );
            } else if cmd.is_local_accept() {
                pool_tx
                    .evidence_mut()
                    .add_shard_group(local_shard_group)
                    .set_accept_qc(justify_id);
                debug!(
                    target: LOG_TARGET,
                    "🔍 Updating evidence for LocalAccept command in block {} for transaction {}. {}",
                    block,
                    atom.id(),
                    pool_tx.evidence()
                );
            } else {
                // Nothing
            }

            // Set readiness
            if !pool_tx.is_ready() && pool_tx.is_ready_for_pending_stage(local_shard_group) {
                debug!(
                    target: LOG_TARGET,
                    "✅ Justified block: Setting READY for transaction {} in block {}",
                    atom.id(),
                    block,
                );
                pool_tx.set_ready(true);
            }

            change_set.set_next_transaction_update(pool_tx)?;
        }

        timer.with_iterations(num_applicable_commands);
        Ok(())
    }

    // Update the pending transaction pool state for the chain of newly justified blocks.
    let mut blocks_to_process = vec![];
    let mut parent_block = new_leaf_block;
    loop {
        let parent = parent_block.get_parent(tx)?;
        if parent.has_justify_qc() {
            break;
        }
        blocks_to_process.push(parent);
        parent_block = blocks_to_process.last().expect("Added above");
    }

    timer.with_iterations(blocks_to_process.len());
    info!(
        target: LOG_TARGET,
        "⚙️ Processing {} blocks in the justified chain",
        blocks_to_process.len(),
    );

    let leaf = new_leaf_block.as_leaf();
    for block in blocks_to_process.iter().rev().chain(iter::once(new_leaf_block)) {
        process_newly_justified_block_inner::<TTx>(tx, block, &leaf, justify_id, local_committee_info, change_set)?;
    }

    Ok(blocks_to_process)
}
