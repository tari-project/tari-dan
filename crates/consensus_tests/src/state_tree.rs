//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

use tari_consensus::hotstuff::{HotStuffError, commit_proofs::state_version_commit_proof};
use tari_consensus_types::{BlockId, Decision};
use tari_ootle_common_types::{
    Epoch,
    InputDeclaration,
    NodeHeight,
    SubstateLockType,
    SubstateVersion,
    optional::Optional,
    shard::Shard,
};
use tari_ootle_storage::{
    ShardScopedTreeStoreReader,
    StateStore,
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
    consensus_models::{
        Block,
        CommittedBlockProof,
        StateVersionProofSource,
        SubstateValueFilterFlags,
        is_state_version_proof_point,
        verify_state_version_leaf,
    },
};
use tari_ootle_transaction::{Network, Transaction, TransactionId};
use tari_state_tree::{
    SPARSE_MERKLE_PLACEHOLDER_HASH,
    SpreadPrefixStateTree,
    Version,
    key_mapper::SpreadPrefixKeyMapper,
    memory_store::MemoryTreeStore,
};

use crate::support::{
    TEST_NUM_PRESHARDS,
    Test,
    TestAddress,
    TestVnDestination,
    build_transaction_from,
    logging::setup_logger,
};

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

/// Commits a few transactions on a single validator in epoch 1, keeping `state_history_length` versions of each
/// shard's history, then stops it. With `consume_outputs`, a later transaction takes down the first one's outputs, so
/// their shards move again.
async fn commit_state_in_epoch_one(state_history_length: Option<u64>, consume_outputs: bool) -> Test {
    setup_logger();
    let mut test = Test::builder()
        .modify_config(|config_mut| {
            config_mut.epoch_end_grace_period = Duration::from_secs(0);
            if let Some(state_history_length) = state_history_length {
                config_mut.state_history_length = state_history_length;
            }
        })
        .modify_consensus_constants(|config| {
            config.pacemaker_block_time = Duration::from_millis(500);
        })
        .add_committee(0, vec!["1"])
        .start()
        .await;
    let (create, _, outputs) = test.send_transaction_to_all(Decision::Commit, 100, 1, 10).await;
    let (other, _, _) = test.send_transaction_to_all(Decision::Commit, 200, 1, 1).await;
    test.start_epoch(Epoch(1)).await;
    commit_transactions(&mut test, &[*create.id(), *other.id()]).await;

    if consume_outputs {
        let inputs = outputs
            .into_iter()
            .map(|output| InputDeclaration::write_versioned(output, SubstateVersion::ZERO))
            .collect::<Vec<_>>();
        let transaction = build_transaction_from(
            Transaction::builder_localnet(Epoch(1))
                .with_inputs(inputs.clone())
                .build_and_seal(&Default::default()),
        );
        test.create_execution_at_destination_for_transaction(
            TestVnDestination::All,
            &transaction,
            Decision::Commit,
            100,
            inputs
                .into_iter()
                .map(|input| (input.substate_id().clone(), SubstateLockType::Write))
                .collect(),
            vec![],
        );
        let consume = *transaction.id();
        test.send_transaction_to_destination(TestVnDestination::All, transaction)
            .await;
        commit_transactions(&mut test, &[consume]).await;
    }
    test.stop();
    test
}

async fn commit_transactions(test: &mut Test, transactions: &[TransactionId]) {
    loop {
        test.on_block_committed().await;
        if transactions
            .iter()
            .all(|id| test.is_transaction_finalized_at_destination(TestVnDestination::All, id))
        {
            return;
        }
        let leaf = test.get_validator(&TestAddress::new("1")).get_leaf_block();
        if leaf.height >= NodeHeight(20) {
            panic!("Not all transactions committed after {} blocks", leaf.height);
        }
    }
}

/// Checks that each committed version of each shard that `is_kept(version, latest)` selects, and no other, has a proof
/// that this node can serve, and that the proof verifies against the commit proof it is served with. Returns the
/// blocks the proofs name.
fn assert_state_versions_are_provable<TTx: StateStoreReadTransaction>(
    tx: &TTx,
    is_kept: impl Fn(Version, Version) -> bool,
) -> HashSet<BlockId> {
    let mut blocks = HashSet::new();
    for shard in TEST_NUM_PRESHARDS.all_shards_iter() {
        let Some(latest) = tx.state_tree_versions_get_latest(shard).unwrap() else {
            continue;
        };
        let proofs = tx.state_version_proofs_get_range(shard, 1, latest).unwrap();
        assert_eq!(
            proofs.iter().map(|p| p.state_version).collect::<Vec<_>>(),
            (1..=latest)
                .filter(|version| is_kept(*version, latest))
                .collect::<Vec<_>>(),
            "{shard} at v{latest} holds the wrong proofs"
        );
        for proof in proofs {
            let StateVersionProofSource::Committed { block_id } = proof.source else {
                panic!("{shard} v{} was not indexed at commit", proof.state_version);
            };
            blocks.insert(block_id);
            let commit_proof = state_version_commit_proof(tx, proof.source)
                .unwrap_or_else(|e| panic!("{shard} v{} cannot be served: {e}", proof.state_version));
            let commit_proof = CommittedBlockProof::from_bytes(&commit_proof).unwrap();
            let mut store = ShardScopedTreeStoreReader::new(tx, shard);
            let shard_root = SpreadPrefixStateTree::new(&mut store)
                .get_root_hash(proof.state_version)
                .unwrap();
            verify_state_version_leaf(
                Network::LocalNet,
                &commit_proof,
                shard,
                proof.state_version,
                &shard_root,
                &proof.shard_root_proof,
            )
            .unwrap();
        }
    }
    blocks
}

/// The block each proof stored for a committed version names.
fn proof_blocks<TTx: StateStoreReadTransaction>(tx: &TTx) -> HashMap<(Shard, Version), BlockId> {
    let mut blocks = HashMap::new();
    for shard in TEST_NUM_PRESHARDS.all_shards_iter() {
        let Some(latest) = tx.state_tree_versions_get_latest(shard).unwrap() else {
            continue;
        };
        for proof in tx.state_version_proofs_get_range(shard, 1, latest).unwrap() {
            if let StateVersionProofSource::Committed { block_id } = proof.source {
                blocks.insert((shard, proof.state_version), block_id);
            }
        }
    }
    blocks
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_committed_state_version_is_provable_against_its_block() {
    let test = commit_state_in_epoch_one(None, false).await;
    test.get_validator(&TestAddress::new("1"))
        .state_store
        .with_read_tx(|tx| {
            let blocks = assert_state_versions_are_provable(tx, |_, _| true);
            assert!(!blocks.is_empty(), "the test committed no state");
            Ok::<_, StorageError>(())
        })
        .unwrap();
}

/// A syncing peer asks for proofs of proof points from the start of a shard's history, and a down proof record is
/// proved up at a recent version, long after epoch GC has pruned the blocks that produced them. Every other proof goes
/// once its version leaves the shard's version window, with the commit proof of a block no remaining proof names.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn committed_state_versions_stay_provable_after_their_blocks_are_pruned() {
    const STATE_HISTORY: u64 = 0;
    let test = commit_state_in_epoch_one(Some(STATE_HISTORY), true).await;
    let store = &test.get_validator(&TestAddress::new("1")).state_store;
    let committed = store
        .with_read_tx(|tx| Ok::<_, StorageError>(proof_blocks(tx)))
        .unwrap();

    // Far enough past epoch 1 that every retention window has passed.
    store.with_write_tx(|tx| tx.epoch_cleanup(Epoch(100))).unwrap();
    store
        .with_write_tx(|tx| tx.state_tree_nodes_clear_stale(TEST_NUM_PRESHARDS, usize::MAX))
        .unwrap();

    store
        .with_read_tx(|tx| {
            let kept = assert_state_versions_are_provable(tx, |version, latest| {
                is_state_version_proof_point(version) || version >= latest.saturating_sub(STATE_HISTORY)
            });
            for block_id in &kept {
                assert!(
                    Block::get(tx, block_id).optional()?.is_none(),
                    "block {block_id} was not pruned, so the test does not exercise a pruned block"
                );
            }
            assert!(
                proof_blocks(tx).len() < committed.len(),
                "no version left its shard's window, so the test prunes nothing"
            );
            for block_id in committed.values().collect::<HashSet<_>>() {
                assert_eq!(
                    tx.block_commit_proofs_get(block_id).optional()?.is_some(),
                    kept.contains(block_id),
                    "block {block_id}'s commit proof must be stored exactly while a proof names it"
                );
            }
            Ok::<_, StorageError>(())
        })
        .unwrap();
}
