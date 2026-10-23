//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use indexmap::IndexMap;
use tari_indexer_client::types::EpochCheckpointEntry;
use tari_ootle_common_types::{Epoch, StateVersion, shard::Shard};
use tari_ootle_light_client::{LightClientError, TrustedValidatorSet, VerifiedCommittee, test_fixtures::*};
use tari_ootle_storage::consensus_models::{EpochCheckpoint, TreeRootSummary};
use tari_state_tree::{SPARSE_MERKLE_PLACEHOLDER_HASH, TransitionAccumulator};

/// Validator sets of epochs 1 to 3.
fn sets() -> Vec<ValidatorSetFixture> {
    (1..=3).map(ValidatorSetFixture::new).collect()
}

/// The set of epoch 2, pinned.
fn trusted(sets: &[ValidatorSetFixture]) -> TrustedValidatorSet {
    TrustedValidatorSet::anchor_pinned(
        NETWORK,
        walk_shard(),
        Epoch(2),
        &sets[1].hash(),
        &sets[1].committee(low()),
    )
    .unwrap()
}

fn high_committee(sets: &[ValidatorSetFixture]) -> VerifiedCommittee {
    trusted(sets).verify_committee(&sets[1].committee(high())).unwrap()
}

/// The checkpoint of `high` ending epoch 2, with shard 3 at state version 7.
fn high_checkpoint(sets: &[ValidatorSetFixture]) -> EpochCheckpointEntry {
    checkpoint_entry(CheckpointSpec {
        shard_group: high(),
        signers: &sets[1].high,
        next_epoch_committee: Some(sets[2].committee(high())),
        shard_state_versions: vec![(Shard::from_u32(3), 7)],
        ..CheckpointSpec::handover(&sets[1], &sets[2])
    })
}

#[test]
fn a_verified_checkpoint_reports_the_state_versions_it_commits() {
    let sets = sets();
    let checkpoint = high_committee(&sets)
        .verify_checkpoint(&high_checkpoint(&sets))
        .unwrap();

    assert_eq!(checkpoint.epoch(), Epoch(2));
    assert_eq!(checkpoint.shard_group(), high());
    assert_eq!(
        checkpoint.shard_state_version(Shard::from_u32(3)),
        Some(StateVersion::from(7))
    );
    assert_eq!(
        checkpoint.shard_state_version(Shard::from_u32(4)),
        Some(StateVersion::zero())
    );
    assert_eq!(checkpoint.shard_state_version(Shard::from_u32(1)), None);
}

#[test]
fn a_checkpoint_with_a_state_version_its_root_does_not_commit_is_rejected() {
    let sets = sets();
    let entry = high_checkpoint(&sets);
    let signed: EpochCheckpoint = tari_bor::decode(&entry.checkpoint).unwrap();
    let raised = EpochCheckpoint::new(
        signed.proof().clone(),
        IndexMap::from([(Shard::from_u32(3), TreeRootSummary {
            root_hash: SPARSE_MERKLE_PLACEHOLDER_HASH,
            state_version: u64::MAX,
            transition_acc: TransitionAccumulator::zero(),
        })]),
        signed.header_metadata().clone(),
    );
    let entry = EpochCheckpointEntry {
        checkpoint: tari_bor::encode(&raised).unwrap(),
        ..entry
    };

    let err = high_committee(&sets).verify_checkpoint(&entry).unwrap_err();
    assert!(matches!(err, LightClientError::InvalidCheckpoint { .. }), "{err}");
}

#[test]
fn a_checkpoint_the_committee_did_not_sign_is_rejected() {
    let sets = sets();
    let impostors = Signers::new(0xee, 4);
    let entry = checkpoint_entry(CheckpointSpec {
        shard_group: high(),
        signers: &impostors,
        shard_state_versions: vec![(Shard::from_u32(3), 7)],
        ..CheckpointSpec::handover(&sets[1], &sets[2])
    });

    let err = high_committee(&sets).verify_checkpoint(&entry).unwrap_err();
    assert!(matches!(err, LightClientError::InvalidCheckpoint { .. }), "{err}");
}

#[test]
fn a_checkpoint_of_another_shard_group_is_rejected() {
    let sets = sets();
    let entry = checkpoint_entry(CheckpointSpec::handover(&sets[1], &sets[2]));

    let err = high_committee(&sets).verify_checkpoint(&entry).unwrap_err();
    assert!(
        matches!(err, LightClientError::CheckpointShardGroupMismatch { .. }),
        "{err}"
    );
}

#[test]
fn a_checkpoint_of_another_epoch_is_rejected() {
    let sets = sets();
    let entry = checkpoint_entry(CheckpointSpec {
        shard_group: high(),
        signers: &sets[0].high,
        ..CheckpointSpec::handover(&sets[0], &sets[1])
    });

    let err = high_committee(&sets).verify_checkpoint(&entry).unwrap_err();
    assert!(matches!(err, LightClientError::CheckpointEpochMismatch { .. }), "{err}");
}
