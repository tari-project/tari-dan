//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

pub mod helpers;

use std::{collections::BTreeMap, time::Instant};

use helpers::{NETWORK, build_substate_record, create_rocksdb, create_substate_update_batch, num_preshards};
use tari_ootle_common_types::{Epoch, ShardGroup, shard::Shard};
use tari_ootle_storage::{
    ShardScopedTreeStoreWriter,
    StateStore,
    StateStoreWriteTransaction,
    SubstateProofGenerator,
    consensus_models::{Block, SubstateRecord},
    generate_substate_proof,
};
use tari_state_tree::{SpreadPrefixStateTree, SubstateTreeChange};

use crate::helpers::substate_id_seed;

/// Measures what a batch of proofs costs a validator to produce, which is what bounds how large a
/// batch a validator is willing to answer. Run with `--release --nocapture`; ignored by default
/// because it asserts nothing - wall-clock thresholds would only make CI flaky.
#[test]
#[ignore = "timing measurement, run manually"]
fn proof_cost() {
    const BATCH: u32 = 50;
    let (db, _tmp) = create_rocksdb();
    // Worst case for the fixed cost: one committee, so the shard group is every preshard.
    let shard_group = ShardGroup::all_shards(num_preshards());

    let substates = (0..BATCH)
        .map(|seed| build_substate_record(&substate_id_seed(seed << 24), 0, 1))
        .collect::<Vec<_>>();
    let mut by_shard: BTreeMap<Shard, Vec<&SubstateRecord>> = BTreeMap::new();
    for s in &substates {
        by_shard.entry(s.created().in_shard).or_default().push(s);
    }
    println!(
        "{} substates over {} distinct shards, shard group of {}",
        substates.len(),
        by_shard.len(),
        shard_group.len()
    );

    let mut tx = db.create_write_tx().unwrap();
    Block::zero_block(NETWORK, num_preshards()).insert(&mut tx).unwrap();
    for (shard, records) in &by_shard {
        let changes = records.iter().map(|s| SubstateTreeChange::Up {
            id: s.to_versioned_substate_id(),
            value_hash: *s.state_hash(),
        });
        {
            let mut store = ShardScopedTreeStoreWriter::new(&mut tx, *shard);
            SpreadPrefixStateTree::new(&mut store)
                .batch_put_substate_changes(None, 1, changes)
                .unwrap();
        }
        tx.state_tree_shard_versions_set(*shard, 1).unwrap();
    }
    tx.substates_commit_batch(create_substate_update_batch(Epoch::zero(), &substates))
        .unwrap();
    tx.commit().unwrap();

    let tx = db.create_read_tx().unwrap();

    // One-shot per substate: what the batch would cost without hoisting.
    let t = Instant::now();
    for s in &substates {
        generate_substate_proof(&tx, shard_group, &s.to_versioned_substate_id(), num_preshards()).unwrap();
    }
    let one_shot = t.elapsed();

    // Hoisted: one generator for the whole batch.
    let t = Instant::now();
    let mut generator = SubstateProofGenerator::new(&tx, shard_group, num_preshards()).unwrap();
    let construct = t.elapsed();
    let t = Instant::now();
    for s in &substates {
        generator.generate(&s.to_versioned_substate_id()).unwrap().unwrap();
    }
    let per_substate = t.elapsed();

    println!("one-shot x{BATCH}:      {one_shot:?}");
    println!("generator construct: {construct:?}");
    println!("generator x{BATCH}:     {per_substate:?}");
    println!("hoisted total:       {:?}", construct + per_substate);

    // A single-substate request pays the whole fixed cost too.
    let t = Instant::now();
    generate_substate_proof(
        &tx,
        shard_group,
        &substates[0].to_versioned_substate_id(),
        num_preshards(),
    )
    .unwrap();
    println!("one substate alone:  {:?}", t.elapsed());
}
