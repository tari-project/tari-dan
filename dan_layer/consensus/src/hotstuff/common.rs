//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::HashMap,
    ops::{ControlFlow, Deref},
};

use indexmap::IndexMap;
use log::*;
use tari_common::configuration::Network;
use tari_common_types::types::FixedHash;
use tari_dan_common_types::{committee::Committee, shard::Shard, Epoch, NodeAddressable, NodeHeight, ShardGroup};
use tari_dan_storage::{
    consensus_models::{
        Block,
        EpochCheckpoint,
        LeafBlock,
        PendingShardStateTreeDiff,
        QuorumCertificate,
        SubstateChange,
        VersionedStateHashTreeDiff,
    },
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
};
use tari_state_tree::{Hash, JellyfishMerkleTree, StateTreeError};

use crate::{
    hotstuff::{
        substate_store::{ShardScopedTreeStoreReader, ShardedStateTree},
        HotStuffError,
    },
    traits::LeaderStrategy,
};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::common";

/// The value that fees are divided by to determine the amount of fees to burn. 0 means no fees are burned.
/// This is a placeholder for the fee exhaust consensus constant so that we know where it's used later.
pub const EXHAUST_DIVISOR: u64 = 20; // 5%

/// Calculates the dummy block required to reach the new height and returns the last dummy block (parent for next
/// proposal).
pub fn calculate_last_dummy_block<TAddr: NodeAddressable, TLeaderStrategy: LeaderStrategy<TAddr>>(
    network: Network,
    epoch: Epoch,
    shard_group: ShardGroup,
    high_qc: &QuorumCertificate,
    parent_merkle_root: FixedHash,
    new_height: NodeHeight,
    leader_strategy: &TLeaderStrategy,
    local_committee: &Committee<TAddr>,
    parent_timestamp: u64,
    parent_base_layer_block_height: u64,
    parent_base_layer_block_hash: FixedHash,
) -> Option<LeafBlock> {
    let mut dummy = None;
    with_dummy_blocks(
        network,
        epoch,
        shard_group,
        high_qc,
        parent_merkle_root,
        new_height,
        leader_strategy,
        local_committee,
        parent_timestamp,
        parent_base_layer_block_height,
        parent_base_layer_block_hash,
        |dummy_block| {
            dummy = Some(dummy_block.as_leaf_block());
            ControlFlow::Continue(())
        },
    );

    dummy
}

/// Calculates the dummy block required to reach the new height
pub fn calculate_dummy_blocks<TAddr: NodeAddressable, TLeaderStrategy: LeaderStrategy<TAddr>>(
    candidate_block: &Block,
    justify_block: &Block,
    leader_strategy: &TLeaderStrategy,
    local_committee: &Committee<TAddr>,
) -> Vec<Block> {
    let mut dummies = Vec::new();
    with_dummy_blocks(
        candidate_block.network(),
        justify_block.epoch(),
        justify_block.shard_group(),
        candidate_block.justify(),
        *justify_block.merkle_root(),
        candidate_block.height(),
        leader_strategy,
        local_committee,
        justify_block.timestamp(),
        justify_block.base_layer_block_height(),
        *justify_block.base_layer_block_hash(),
        |dummy_block| {
            if dummy_block.id() == candidate_block.parent() {
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

fn with_dummy_blocks<TAddr, TLeaderStrategy, F>(
    network: Network,
    epoch: Epoch,
    shard_group: ShardGroup,
    high_qc: &QuorumCertificate,
    parent_merkle_root: FixedHash,
    new_height: NodeHeight,
    leader_strategy: &TLeaderStrategy,
    local_committee: &Committee<TAddr>,
    parent_timestamp: u64,
    parent_base_layer_block_height: u64,
    parent_base_layer_block_hash: FixedHash,
    mut callback: F,
) where
    TAddr: NodeAddressable,
    TLeaderStrategy: LeaderStrategy<TAddr>,
    F: FnMut(Block) -> ControlFlow<()>,
{
    let mut parent_block = high_qc.as_leaf_block();
    let mut current_height = high_qc.block_height() + NodeHeight(1);
    if current_height > new_height {
        warn!(
            target: LOG_TARGET,
            "BUG: 🍼 no dummy blocks to calculate. current height {} is greater than new height {}",
            current_height,
            new_height,
        );
        return;
    }

    debug!(
        target: LOG_TARGET,
        "🍼 calculating dummy blocks from {} to {}",
        current_height,
        new_height,
    );
    loop {
        let leader = leader_strategy.get_leader_public_key(local_committee, current_height);
        let dummy_block = Block::dummy_block(
            network,
            *parent_block.block_id(),
            leader.clone(),
            current_height,
            high_qc.clone(),
            epoch,
            shard_group,
            parent_merkle_root,
            parent_timestamp,
            parent_base_layer_block_height,
            parent_base_layer_block_hash,
        );
        debug!(
            target: LOG_TARGET,
            "🍼 new dummy block: {}",
            dummy_block,
        );
        parent_block = dummy_block.as_leaf_block();

        if callback(dummy_block).is_break() {
            break;
        }

        if current_height == new_height {
            break;
        }
        current_height += NodeHeight(1);
    }
}

pub fn calculate_state_merkle_root<'a, TTx: StateStoreReadTransaction, I: IntoIterator<Item = &'a SubstateChange>>(
    tx: &TTx,
    shard_group: ShardGroup,
    pending_tree_diffs: HashMap<Shard, Vec<PendingShardStateTreeDiff>>,
    changes: I,
) -> Result<(Hash, IndexMap<Shard, VersionedStateHashTreeDiff>), StateTreeError> {
    let mut change_map = IndexMap::new();

    changes.into_iter().for_each(|ch| {
        // Group by shard
        change_map.entry(ch.shard()).or_insert_with(Vec::new).push(ch.into());
    });
    let mut sharded_tree = ShardedStateTree::new(tx).with_pending_diffs(pending_tree_diffs);
    let root_hash = sharded_tree.put_substate_tree_changes(shard_group, change_map)?;

    Ok((root_hash, sharded_tree.into_shard_tree_diffs()))
}

pub(crate) fn create_epoch_checkpoint<TTx>(
    tx: &mut TTx,
    epoch: Epoch,
    shard_group: ShardGroup,
) -> Result<EpochCheckpoint, HotStuffError>
where
    TTx: StateStoreWriteTransaction + Deref,
    TTx::Target: StateStoreReadTransaction,
{
    // Get the last 3 blocks in the previous epoch. These blocks should end the epoch.
    let mut blocks = Block::get_last_n_in_epoch(&**tx, 3, epoch)?;
    if blocks.is_empty() {
        return Err(HotStuffError::StorageError(StorageError::NotFound {
            item: "Block::get_last_n_in_epoch".to_string(),
            key: epoch.to_string(),
        }));
    }

    let commit_block = blocks.pop().unwrap();
    let qcs = blocks.into_iter().map(|b| b.into_justify()).collect();

    // Fetch the state roots of the shards in the shard group
    let mut shard_roots = IndexMap::with_capacity(shard_group.len());
    for shard in shard_group.shard_iter() {
        let Some(version) = tx.state_tree_versions_get_latest(shard)? else {
            // At v0 there have been no state changes
            continue;
        };

        let scoped_store = ShardScopedTreeStoreReader::new(&**tx, shard);
        let jmt = JellyfishMerkleTree::new(&scoped_store);
        let root_hash = jmt
            .get_root_hash(version)
            .map_err(|e| HotStuffError::StateTreeError(e.into()))?;

        shard_roots.insert(shard, root_hash);
    }
    let checkpoint = EpochCheckpoint::new(commit_block, qcs, shard_roots);
    checkpoint.save(tx)?;

    Ok(checkpoint)
}
