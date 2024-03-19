//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use log::*;
use tari_common::configuration::Network;
use tari_common_types::types::FixedHash;
use tari_dan_common_types::{committee::Committee, Epoch, NodeAddressable, NodeHeight};
use tari_dan_storage::consensus_models::{Block, LeafBlock, PendingStateTreeDiff, QuorumCertificate};
use tari_engine_types::{
    hashing::substate_value_hasher32,
    substate::{Substate, SubstateDiff},
};
use tari_state_tree::{
    Hash,
    StagedTreeStore,
    StateHashTreeDiff,
    StateTreeError,
    SubstateChange,
    TreeStoreReader,
    Version,
};

use crate::traits::LeaderStrategy;

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::common";

/// The value that fees are divided by to determine the amount of fees to burn. 0 means no fees are burned.
/// This is a placeholder for the fee exhaust consensus constant so that we know where it's used later.
pub const EXHAUST_DIVISOR: u64 = 20; // 5%

/// Calculates the dummy block required to reach the new height and returns the last dummy block (parent for next
/// proposal).
pub fn calculate_last_dummy_block<TAddr: NodeAddressable, TLeaderStrategy: LeaderStrategy<TAddr>>(
    network: Network,
    epoch: Epoch,
    high_qc: &QuorumCertificate,
    parent_merkle_root: FixedHash,
    new_height: NodeHeight,
    leader_strategy: &TLeaderStrategy,
    local_committee: &Committee<TAddr>,
    parent_timestamp: u64,
    parent_base_layer_block_hash: FixedHash,
) -> Option<LeafBlock> {
    let mut parent_block = high_qc.as_leaf_block();
    let mut current_height = high_qc.block_height() + NodeHeight(1);
    if current_height > new_height {
        warn!(
            target: LOG_TARGET,
            "BUG: 🍼 no dummy blocks to calculate. current height {} is greater than new height {}",
            current_height,
            new_height,
        );
        return None;
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
            parent_merkle_root,
            parent_timestamp,
            parent_base_layer_block_hash,
        );
        debug!(
            target: LOG_TARGET,
            "🍼 new dummy block: {}",
            dummy_block,
        );
        parent_block = dummy_block.as_leaf_block();

        if current_height == new_height {
            break;
        }
        current_height += NodeHeight(1);
    }

    Some(parent_block)
}

pub fn diff_to_substate_changes(diff: &SubstateDiff) -> impl Iterator<Item = SubstateChange> + '_ {
    diff.down_iter()
        .map(|(substate_id, _version)| SubstateChange::Down {
            id: substate_id.clone(),
        })
        .chain(diff.up_iter().map(move |(substate_id, value)| SubstateChange::Up {
            id: substate_id.clone(),
            value_hash: hash_substate(value),
        }))
}

pub fn hash_substate(substate: &Substate) -> FixedHash {
    substate_value_hasher32().chain(substate).result().into_array().into()
}

pub fn calculate_state_merkle_diff<TTx: TreeStoreReader<Version>, I: IntoIterator<Item = SubstateChange>>(
    tx: &TTx,
    current_version: Version,
    next_version: Version,
    pending_tree_updates: Vec<PendingStateTreeDiff>,
    substate_changes: I,
) -> Result<(Hash, StateHashTreeDiff), StateTreeError> {
    debug!(
        target: LOG_TARGET,
        "Calculating state merkle diff from version {} to {} with {} update(s)",
        current_version,
        next_version,
        pending_tree_updates.len(),
    );
    let mut store = StagedTreeStore::new(tx);
    store.apply_ordered_diffs(pending_tree_updates.into_iter().map(|diff| diff.diff));
    let mut state_tree = tari_state_tree::SpreadPrefixStateTree::new(&mut store);
    let state_root = state_tree.put_substate_changes(current_version, next_version, substate_changes)?;
    Ok((state_root, store.into_diff()))
}
