//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use log::*;
use tari_common::configuration::Network;
use tari_common_types::types::FixedHash;
use tari_dan_common_types::{committee::Committee, Epoch, NodeAddressable, NodeHeight};
use tari_dan_storage::consensus_models::{Block, BlockFee, LeafBlock, PendingStateTreeDiff, QuorumCertificate};
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

pub fn calculate_block_fee(total_block_fee: u64, total_num_involved_shards: u64, exhaust_divisor: u64) -> BlockFee {
    // If there are no involved shards (because there were no Accept commands), then there is no leader fee or burn
    if total_num_involved_shards == 0 {
        return BlockFee {
            leader_fee: 0,
            global_exhaust_burn: 0,
        };
    }

    let target_burn = total_block_fee.checked_div(exhaust_divisor).unwrap_or(0);
    let block_fee_after_burn = total_block_fee - target_burn;

    let mut leader_fee = block_fee_after_burn / total_num_involved_shards;
    // The extra amount that is burnt from dividing the number of shards involved
    let remainder_burn = block_fee_after_burn % total_num_involved_shards;

    // Adjust the leader fee to account for the remainder
    // If the remainder accounts for an extra burn of greater than half the number of involved shards, we
    // give each validator an extra 1 in fees if enough fees are available, burning less than the exhaust target.
    // Otherwise, we burn a little more than/equal to the exhaust target.
    let actual_burn = if remainder_burn > 0 &&
        // If the div floor burn accounts for 1 less fee for more than half of number of shards, and ...
        remainder_burn >= total_num_involved_shards / 2 &&
        // ... if there are enough fees to pay out an additional 1 to all shards
        (leader_fee + 1) * total_num_involved_shards <= total_block_fee
    {
        // Pay each leader 1 more
        leader_fee += 1;

        // We burn a little less due to the remainder
        target_burn.saturating_sub(total_num_involved_shards - remainder_burn)
    } else {
        // We burn a little more due to the remainder
        target_burn + remainder_burn
    };

    BlockFee {
        leader_fee,
        global_exhaust_burn: actual_burn,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod calculate_block_fee {
        use super::*;

        fn check_calculate_block_fee(
            total_block_fee: u64,
            total_num_involved_shards: u64,
            exhaust_divisor: u64,
        ) -> BlockFee {
            let block_fee = calculate_block_fee(total_block_fee, total_num_involved_shards, exhaust_divisor);
            // Total payable fee + burn is always equal to the total block fee
            assert_eq!(
                block_fee.leader_fee * total_num_involved_shards + block_fee.global_exhaust_burn,
                total_block_fee
            );

            let deviation_from_target_burn = block_fee.global_exhaust_burn as f32 -
                (total_block_fee.checked_div(exhaust_divisor).unwrap_or(0) as f32);
            assert!(
                deviation_from_target_burn.abs() <= (total_num_involved_shards as f32 / 2f32).ceil(),
                "Deviation from target burn is too high: {} (target: {}, actual: {}, num_shards: {}, divisor: {})",
                deviation_from_target_burn,
                total_block_fee.checked_div(exhaust_divisor).unwrap_or(0),
                block_fee.global_exhaust_burn,
                total_num_involved_shards,
                exhaust_divisor
            );

            block_fee
        }

        #[test]
        fn it_calculates_the_correct_leader_fee() {
            // This will fail if exhaust_divisor changes
            let fee = check_calculate_block_fee(100, 1, EXHAUST_DIVISOR);
            assert_eq!(fee.leader_fee, 95);
            assert_eq!(fee.global_exhaust_burn, 5);

            let fee = check_calculate_block_fee(100, 1, 10);
            assert_eq!(fee.leader_fee, 90);
            assert_eq!(fee.global_exhaust_burn, 10);

            let fee = check_calculate_block_fee(100, 2, 0);
            assert_eq!(fee.leader_fee, 50);
            assert_eq!(fee.global_exhaust_burn, 0);

            let fee = check_calculate_block_fee(100, 2, 10);
            assert_eq!(fee.leader_fee, 45);
            assert_eq!(fee.global_exhaust_burn, 10);

            let fee = check_calculate_block_fee(100, 3, 0);
            assert_eq!(fee.leader_fee, 33);
            // Even with no exhaust, we still burn 1 due to integer div floor
            assert_eq!(fee.global_exhaust_burn, 1);

            let fee = check_calculate_block_fee(100, 3, 10);
            assert_eq!(fee.leader_fee, 30);
            assert_eq!(fee.global_exhaust_burn, 10);

            let fee = check_calculate_block_fee(98, 3, 10);
            assert_eq!(fee.leader_fee, 30);
            assert_eq!(fee.global_exhaust_burn, 8);

            let fee = check_calculate_block_fee(98, 3, 21);
            assert_eq!(fee.leader_fee, 32);
            // target burn is 4, but the remainder burn is 5, so we give 1 more to the leaders and burn 2
            assert_eq!(fee.global_exhaust_burn, 2);

            // Target burn is 8, and the remainder burn is 8, so we burn 8
            let fee = check_calculate_block_fee(98, 10, 10);
            assert_eq!(fee.leader_fee, 9);
            assert_eq!(fee.global_exhaust_burn, 8);

            let fee = check_calculate_block_fee(19802, 45, 20);
            assert_eq!(fee.leader_fee, 418);
            assert_eq!(fee.global_exhaust_burn, 992);
        }
    }
}
