//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

pub mod helpers;

use helpers::{build_substate_record, create_rocksdb, create_substate_update_batch};
use tari_engine_types::substate::SubstateId;
use tari_ootle_common_types::{Epoch, VersionedSubstateIdRef, optional::Optional, shard::Shard};
use tari_ootle_storage::{
    StateStore,
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    consensus_models::{SubstateTransition, SubstateUpdateBatch, SubstateValueOrHash},
};
use tari_state_tree::Version;
use tari_template_lib::types::{ComponentAddress, ObjectKey};
use tari_validator_rollback::storage::substates_rewind_to_state_version;

use crate::helpers::num_preshards;

fn substate_id(seed: u8) -> SubstateId {
    SubstateId::Component(ComponentAddress::from_array([seed; ObjectKey::LENGTH]))
}

fn shard_for(id: &SubstateId, version: u64) -> Shard {
    VersionedSubstateIdRef::new(id, version).to_shard(num_preshards())
}

#[test]
fn rewind_deletes_upped_records_and_restores_downed() {
    let (db, _tmp) = create_rocksdb();

    let a = substate_id(1);
    let shard = shard_for(&a, 0);

    // Commit #1 (sv=1, epoch 0): create a@v0.
    let a_v0 = build_substate_record(&a, 0, 1);
    db.with_write_tx(|tx| {
        tx.substates_commit_batch(create_substate_update_batch(Epoch(0), [&a_v0]))
            .unwrap();
        tx.state_tree_shard_versions_set(shard, 1).unwrap();
        Ok::<_, tari_ootle_storage::StorageError>(())
    })
    .unwrap();

    // Commit #2 (sv=3, epoch 1): down a@v0 and up a@v1 — simulating an update.
    let a_v1 = {
        let mut r = build_substate_record(&a, 1, 3);
        r.created.at_epoch = Epoch(1);
        r
    };
    db.with_write_tx(|tx| {
        let mut batch = SubstateUpdateBatch::new(helpers::NETWORK, Epoch(1));
        batch.with_transition(shard, 3).push(SubstateTransition::Down {
            id: a_v0.to_versioned_substate_id(),
        });
        batch.with_transition(shard, 3).push(SubstateTransition::Up {
            id: a.clone(),
            version: 1,
            substate_or_hash: a_v1
                .substate_value()
                .cloned()
                .map(|v| SubstateValueOrHash::Value(Box::new(v)))
                .unwrap(),
        });
        tx.substates_commit_batch(batch).unwrap();
        tx.state_tree_shard_versions_set(shard, 3).unwrap();
        Ok::<_, tari_ootle_storage::StorageError>(())
    })
    .unwrap();

    // Pre-rewind: head points at v1.
    db.with_read_tx(|tx| {
        let (v, is_up) = tx.substates_get_max_version_for_substate(&a).unwrap();
        assert_eq!((v, is_up), (1, true), "a head should be v1 up");
        Ok::<_, tari_ootle_storage::StorageError>(())
    })
    .unwrap();

    // Rewind to state_version 1.
    let stats = db
        .with_write_tx(|tx| substates_rewind_to_state_version(tx, shard, 1))
        .unwrap();
    assert_eq!(stats.transitions_processed, 1, "only sv=3 had a transition record");
    assert_eq!(stats.substates_created_deleted, 1, "a@v1 should be deleted");
    assert_eq!(stats.substates_destroyed_restored, 1, "a@v0 should be restored to up");
    assert_eq!(stats.heads_updated, 1);

    db.with_read_tx(|tx| {
        // a@v0 back to up.
        let got = tx.substates_get(&a_v0.to_substate_address()).unwrap();
        assert!(got.is_up(), "a@v0 should be up after rewind");

        // a@v1 deleted.
        let got = tx.substates_get(&a_v1.to_substate_address()).optional().unwrap();
        assert!(got.is_none(), "a@v1 should be deleted after rewind");

        // Head rebuilt to v0 up.
        let (v, is_up) = tx.substates_get_max_version_for_substate(&a).unwrap();
        assert_eq!((v, is_up), (0, true), "a head should revert to v0 up");
        Ok::<_, tari_ootle_storage::StorageError>(())
    })
    .unwrap();
}

#[test]
fn rewind_past_creation_deletes_head() {
    let (db, _tmp) = create_rocksdb();

    let a = substate_id(1);
    let shard = shard_for(&a, 0);
    // Create a@v0 at state_version 2.
    let a_v0 = {
        let mut r = build_substate_record(&a, 0, 2);
        r.created.at_state_version = 2;
        r
    };
    db.with_write_tx(|tx| {
        tx.substates_commit_batch(create_substate_update_batch(Epoch(0), [&a_v0]))
            .unwrap();
        tx.state_tree_shard_versions_set(shard, 2).unwrap();
        Ok::<_, tari_ootle_storage::StorageError>(())
    })
    .unwrap();

    // Rewind to state_version 1 — a was created at sv=2, so it should vanish.
    let stats = db
        .with_write_tx(|tx| substates_rewind_to_state_version(tx, shard, 1))
        .unwrap();
    assert_eq!(stats.substates_created_deleted, 1);
    assert_eq!(stats.heads_updated, 1);

    db.with_read_tx(|tx| {
        let got = tx.substates_get(&a_v0.to_substate_address()).optional().unwrap();
        assert!(got.is_none(), "a@v0 should be deleted");
        let head = tx.substates_get_max_version_for_substate(&a).optional().unwrap();
        assert!(head.is_none(), "head entry for a should be deleted");
        Ok::<_, tari_ootle_storage::StorageError>(())
    })
    .unwrap();
}

#[test]
fn rewind_noop_when_target_at_or_above_current() {
    let (db, _tmp) = create_rocksdb();
    let a = substate_id(1);
    let shard = shard_for(&a, 0);
    let a_v0 = build_substate_record(&a, 0, 1);
    db.with_write_tx(|tx| {
        tx.substates_commit_batch(create_substate_update_batch(Epoch(0), [&a_v0]))
            .unwrap();
        tx.state_tree_shard_versions_set(shard, 1).unwrap();
        Ok::<_, tari_ootle_storage::StorageError>(())
    })
    .unwrap();

    // Target >= current: nothing to do.
    let stats = db
        .with_write_tx(|tx| substates_rewind_to_state_version(tx, shard, 5))
        .unwrap();
    assert_eq!(stats.transitions_processed, 0);
    assert_eq!(stats.substates_created_deleted, 0);
    assert_eq!(stats.substates_destroyed_restored, 0);
    assert_eq!(stats.heads_updated, 0);

    // Substate still present.
    db.with_read_tx(|tx| {
        let got = tx.substates_get(&a_v0.to_substate_address()).unwrap();
        assert!(got.is_up());
        Ok::<_, tari_ootle_storage::StorageError>(())
    })
    .unwrap();
}

#[test]
fn rewind_preserves_other_shards() {
    let (db, _tmp) = create_rocksdb();

    // Pick two substates on different shards by scanning seeds.
    let mut candidate_a: Option<SubstateId> = None;
    let mut candidate_b: Option<SubstateId> = None;
    for seed in 1u8..=255u8 {
        let id = substate_id(seed);
        let s = shard_for(&id, 0);
        if candidate_a.is_none() {
            candidate_a = Some(id);
            continue;
        }
        if s != shard_for(candidate_a.as_ref().unwrap(), 0) {
            candidate_b = Some(id);
            break;
        }
    }
    let a = candidate_a.expect("could not find substate A");
    let b = candidate_b.expect("could not find a second substate on a different shard");
    let shard_a = shard_for(&a, 0);
    let shard_b = shard_for(&b, 0);
    assert_ne!(shard_a, shard_b);

    let a_v0 = {
        let mut r = build_substate_record(&a, 0, 3);
        r.created.at_state_version = 3;
        r
    };
    let b_v0 = {
        let mut r = build_substate_record(&b, 0, 3);
        r.created.at_state_version = 3;
        r
    };
    db.with_write_tx(|tx| {
        tx.substates_commit_batch(create_substate_update_batch(Epoch(0), [&a_v0, &b_v0]))
            .unwrap();
        tx.state_tree_shard_versions_set(shard_a, 3).unwrap();
        tx.state_tree_shard_versions_set(shard_b, 3).unwrap();
        Ok::<_, tari_ootle_storage::StorageError>(())
    })
    .unwrap();

    // Rewind only shard_a.
    let stats = db
        .with_write_tx(|tx| substates_rewind_to_state_version(tx, shard_a, 1))
        .unwrap();
    assert_eq!(stats.substates_created_deleted, 1);

    // shard_b's substate is untouched.
    db.with_read_tx(|tx| {
        let got = tx.substates_get(&b_v0.to_substate_address()).unwrap();
        assert!(got.is_up());
        let (v, is_up) = tx.substates_get_max_version_for_substate(&b).unwrap();
        assert_eq!((v, is_up), (0, true));

        // a's substate is gone.
        assert!(
            tx.substates_get(&a_v0.to_substate_address())
                .optional()
                .unwrap()
                .is_none()
        );
        Ok::<_, tari_ootle_storage::StorageError>(())
    })
    .unwrap();

    // Silence unused-version warning.
    let _ = Version::default();
}
