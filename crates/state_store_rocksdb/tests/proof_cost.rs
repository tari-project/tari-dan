//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

pub mod helpers;

use std::{collections::BTreeMap, time::Instant};

use helpers::{PROOF_TEST_TREE_VERSION, build_substate_record, commit_substates, create_rocksdb, num_preshards};
use tari_ootle_common_types::{ShardGroup, shard::Shard};
use tari_ootle_storage::{StateStore, SubstateProofGenerator, consensus_models::SubstateRecord};

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
        .map(|seed| build_substate_record(&substate_id_seed(seed << 24), 0, PROOF_TEST_TREE_VERSION))
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

    commit_substates(&db, &substates);

    let tx = db.create_read_tx().unwrap();

    // One generator per substate: what the batch would cost without hoisting.
    let t = Instant::now();
    for s in &substates {
        SubstateProofGenerator::new(&tx, shard_group, num_preshards())
            .unwrap()
            .generate(&s.to_versioned_substate_id())
            .unwrap()
            .unwrap();
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
    SubstateProofGenerator::new(&tx, shard_group, num_preshards())
        .unwrap()
        .generate(&substates[0].to_versioned_substate_id())
        .unwrap()
        .unwrap();
    println!("one substate alone:  {:?}", t.elapsed());
}
