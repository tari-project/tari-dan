//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::time::Duration;

use tari_consensus::hotstuff::HotStuffError;
use tari_consensus_types::Decision;
use tari_ootle_common_types::{Epoch, NodeHeight, optional::Optional};
use tari_ootle_storage::{StateStore, StateStoreReadTransaction, consensus_models::SubstateValueFilterFlags};
use tari_ootle_transaction::Network;
use tari_state_tree::{
    SPARSE_MERKLE_PLACEHOLDER_HASH,
    key_mapper::SpreadPrefixKeyMapper,
    memory_store::MemoryTreeStore,
};

use crate::support::{TEST_NUM_PRESHARDS, Test, TestAddress, logging::setup_logger};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn check_state_transitions() {
    setup_logger();
    let mut test = Test::builder()
        .modify_config(|config_mut| {
            // Epoch change ASAP
            config_mut.epoch_end_grace_period = Duration::from_secs(0);
        })
        .modify_consensus_constants(|config| {
            config.pacemaker_block_time = Duration::from_millis(500);
        })
        .add_committee(0, vec!["1"])
        .start()
        .await;
    let _ignore = test.send_transaction_to_all(Decision::Commit, 100, 1, 10).await;
    let _ignore = test.send_transaction_to_all(Decision::Commit, 200, 1, 1).await;
    let _ignore = test.send_transaction_to_all(Decision::Commit, 1, 1, 1).await;
    let _ignore = test.send_transaction_to_all(Decision::Commit, 100, 1, 10).await;
    test.start_epoch(Epoch(1)).await;

    loop {
        test.on_block_committed().await;

        if test.is_transaction_pool_empty() {
            break;
        }
        let leaf = test.get_validator(&TestAddress::new("1")).get_leaf_block();
        if leaf.height >= NodeHeight(10) {
            panic!("Not all transactions committed after {} blocks", leaf.height);
        }
    }
    // The budget below is relative to where the loop above left off: a commit trails the leaf by a
    // three-chain, so the first block committed in Epoch(2) always arrives several blocks after the
    // epoch starts.
    let epoch_2_deadline = test.get_validator(&TestAddress::new("1")).get_leaf_block().height + NodeHeight(10);
    test.start_epoch(Epoch(2)).await;
    loop {
        let (_, _, epoch, height) = test.on_block_committed().await;

        if epoch == Epoch(2) {
            break;
        }
        if height >= epoch_2_deadline {
            panic!("Epoch(2) not reached by block {height}");
        }
    }

    test.stop();

    test.get_validator(&TestAddress::new("1"))
        .state_store
        .with_read_tx(|tx| {
            let checkpoint = tx
                .epoch_checkpoint_get_all_from_epoch(Epoch(1), 1)
                .unwrap()
                .pop()
                .unwrap();

            for shard in TEST_NUM_PRESHARDS.all_shards_iter() {
                let mut all_transitions = vec![];
                let mut next_state_version = 1;
                while let Some(transitions) = tx
                    .state_transitions_get_starting_at(
                        shard,
                        next_state_version,
                        SubstateValueFilterFlags::all_substates(),
                    )
                    .optional()
                    .unwrap()
                {
                    if transitions.epoch > checkpoint.epoch() {
                        break;
                    }

                    next_state_version = transitions.state_version + 1;
                    all_transitions.push(transitions);
                }
                log::info!(
                    "Shard {}: Found {} transitions until state version {}",
                    shard,
                    all_transitions.len(),
                    next_state_version
                );

                let shard_root = checkpoint.get_shard_root(shard);
                // No state changes
                if shard_root == SPARSE_MERKLE_PLACEHOLDER_HASH {
                    assert!(
                        all_transitions.is_empty(),
                        "Shard {} should have no state transitions",
                        shard
                    );
                } else {
                    assert!(
                        !all_transitions.is_empty(),
                        "Shard {} should have state transitions",
                        shard
                    );
                }

                let mut store = MemoryTreeStore::new();
                let mut tree = tari_state_tree::StateTree::<_, SpreadPrefixKeyMapper>::new(&mut store);
                let values = all_transitions.iter().flat_map(|t| {
                    t.updates
                        .iter()
                        .map(move |u| u.to_tree_change(Network::LocalNet, t.epoch))
                });
                let root = tree.put_substate_changes(None, 1, values).unwrap();
                assert_eq!(root, shard_root, "Shard {} root hash mismatch", shard);
            }

            Ok::<_, HotStuffError>(())
        })
        .unwrap();
    test.assert_clean_shutdown().await;
}
