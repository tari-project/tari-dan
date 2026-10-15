//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

mod support;

use ootle_network::Network;
use support::*;
use tari_ootle_common_types::{Epoch, ValidatorSetHash};
use tari_ootle_light_client::{LightClientError, TrustedValidatorSet, WalkPlan, plan_walk};

/// Validator sets of epochs 1 to 4, each with fresh committees.
fn sets() -> Vec<ValidatorSetFixture> {
    (1..=4).map(ValidatorSetFixture::new).collect()
}

/// The set of epoch 2, trusted on first use of the checkpoint ending epoch 1.
fn anchored(sets: &[ValidatorSetFixture]) -> TrustedValidatorSet {
    let entry = checkpoint_entry(CheckpointSpec::handover(&sets[0], &sets[1]));
    TrustedValidatorSet::anchor_on_first_use(NETWORK, walk_shard(), &entry).unwrap()
}

#[test]
fn first_use_trusts_the_set_the_latest_checkpoint_names() {
    let sets = sets();
    let trusted = anchored(&sets);

    assert_eq!(trusted.epoch(), Epoch(2));
    assert_eq!(trusted.anchor_epoch(), Epoch(2));
    assert_eq!(*trusted.validator_set_hash(), sets[1].hash());
    assert_eq!(trusted.walk_committee().shard_group(), low());
    assert_eq!(
        trusted.walk_committee().members().len(),
        sets[1].low.keys.len(),
        "the walk committee is the proved one"
    );
}

#[test]
fn a_pinned_anchor_trusts_the_pinned_set() {
    let sets = sets();
    let trusted = TrustedValidatorSet::anchor_pinned(
        NETWORK,
        walk_shard(),
        Epoch(3),
        &sets[2].hash(),
        &sets[2].committee(low()),
    )
    .unwrap();
    assert_eq!(trusted.epoch(), Epoch(3));

    let err = TrustedValidatorSet::anchor_pinned(
        NETWORK,
        walk_shard(),
        Epoch(3),
        &sets[2].hash(),
        &sets[3].committee(low()),
    )
    .unwrap_err();
    assert!(matches!(err, LightClientError::CommitteeEpochMismatch { .. }), "{err}");
}

#[test]
fn it_walks_each_handover_to_the_latest_set() {
    let sets = sets();
    let mut trusted = anchored(&sets);
    for window in sets.windows(2).skip(1) {
        trusted = trusted
            .advance(&checkpoint_entry(CheckpointSpec::handover(&window[0], &window[1])))
            .unwrap();
    }

    assert_eq!(trusted.epoch(), Epoch(4));
    assert_eq!(*trusted.validator_set_hash(), sets[3].hash());
    assert_eq!(trusted.anchor_epoch(), Epoch(2));
}

#[test]
fn it_verifies_any_committee_of_the_trusted_set() {
    let sets = sets();
    let trusted = anchored(&sets);

    let high_committee = trusted.verify_committee(&sets[1].committee(high())).unwrap();
    assert_eq!(high_committee.shard_group(), high());

    let err = trusted.verify_committee(&sets[2].committee(high())).unwrap_err();
    assert!(matches!(err, LightClientError::CommitteeEpochMismatch { .. }), "{err}");
}

#[test]
fn a_checkpoint_the_trusted_committee_did_not_sign_is_rejected() {
    let sets = sets();
    let trusted = anchored(&sets);
    let impostors = Signers::new(0xee, 4);
    let entry = checkpoint_entry(CheckpointSpec {
        signers: &impostors,
        ..CheckpointSpec::handover(&sets[1], &sets[2])
    });

    let err = trusted.advance(&entry).unwrap_err();
    assert!(matches!(err, LightClientError::InvalidCheckpoint { .. }), "{err}");
}

#[test]
fn a_checkpoint_stripped_of_its_preimage_is_rejected() {
    let sets = sets();
    let trusted = anchored(&sets);
    let entry = checkpoint_entry(CheckpointSpec {
        carry_preimage: false,
        ..CheckpointSpec::handover(&sets[1], &sets[2])
    });

    let err = trusted.advance(&entry).unwrap_err();
    assert!(matches!(err, LightClientError::NoNextValidatorSet { .. }), "{err}");

    let first_use = TrustedValidatorSet::anchor_on_first_use(NETWORK, walk_shard(), &entry).unwrap_err();
    assert!(
        matches!(first_use, LightClientError::NoNextValidatorSet { .. }),
        "{first_use}"
    );
}

#[test]
fn a_next_committee_outside_the_named_set_is_rejected() {
    let sets = sets();
    let trusted = anchored(&sets);
    // The checkpoint names epoch 3's set, and the indexer serves a committee of another set for epoch 3.
    let forged_set = ValidatorSetFixture::with_signers(NETWORK, 3, Signers::new(0xf0, 4), Signers::new(0xf1, 4));
    let entry = checkpoint_entry(CheckpointSpec {
        next_epoch_committee: Some(forged_set.committee(low())),
        ..CheckpointSpec::handover(&sets[1], &sets[2])
    });

    let err = trusted.advance(&entry).unwrap_err();
    assert!(matches!(err, LightClientError::CommitteeNotInSet(_)), "{err}");
}

#[test]
fn a_checkpoint_without_the_next_committee_is_rejected() {
    let sets = sets();
    let trusted = anchored(&sets);
    let entry = checkpoint_entry(CheckpointSpec {
        next_epoch_committee: None,
        ..CheckpointSpec::handover(&sets[1], &sets[2])
    });

    let err = trusted.advance(&entry).unwrap_err();
    assert!(matches!(err, LightClientError::MissingNextCommittee { .. }), "{err}");
}

#[test]
fn a_next_committee_of_another_shard_group_is_rejected() {
    let sets = sets();
    let trusted = anchored(&sets);
    let entry = checkpoint_entry(CheckpointSpec {
        next_epoch_committee: Some(sets[2].committee(high())),
        ..CheckpointSpec::handover(&sets[1], &sets[2])
    });

    let err = trusted.advance(&entry).unwrap_err();
    assert!(
        matches!(err, LightClientError::CommitteeOutsideWalkedShard { .. }),
        "{err}"
    );
}

#[test]
fn a_checkpoint_of_another_shard_group_is_rejected() {
    let sets = sets();
    let trusted = anchored(&sets);
    let entry = checkpoint_entry(CheckpointSpec {
        shard_group: high(),
        signers: &sets[1].high,
        ..CheckpointSpec::handover(&sets[1], &sets[2])
    });

    let err = trusted.advance(&entry).unwrap_err();
    assert!(
        matches!(err, LightClientError::CheckpointShardGroupMismatch { .. }),
        "{err}"
    );
}

#[test]
fn a_checkpoint_out_of_order_is_rejected() {
    let sets = sets();
    let trusted = anchored(&sets);
    let entry = checkpoint_entry(CheckpointSpec::handover(&sets[2], &sets[3]));

    let err = trusted.advance(&entry).unwrap_err();
    assert!(
        matches!(err, LightClientError::CheckpointEpochMismatch {
            expected: Epoch(2),
            found: Epoch(3)
        }),
        "{err}"
    );
}

#[test]
fn a_checkpoint_of_another_network_is_rejected() {
    let sets = sets();
    let trusted = anchored(&sets);
    let entry = checkpoint_entry(CheckpointSpec {
        network: Network::Esmeralda,
        ..CheckpointSpec::handover(&sets[1], &sets[2])
    });

    let err = trusted.advance(&entry).unwrap_err();
    assert!(matches!(err, LightClientError::WrongNetwork { .. }), "{err}");

    let first_use = TrustedValidatorSet::anchor_on_first_use(Network::Esmeralda, walk_shard(), &entry).unwrap_err();
    assert!(
        matches!(first_use, LightClientError::WrongNetwork { .. }),
        "the committee proof is LocalNet's: {first_use}"
    );
}

#[test]
fn a_committee_without_vote_power_is_never_trusted() {
    let set = ValidatorSetFixture::with_signers(NETWORK, 2, Signers::new(1, 0), Signers::new(2, 4));
    let err = TrustedValidatorSet::anchor_pinned(NETWORK, walk_shard(), Epoch(2), &set.hash(), &set.committee(low()))
        .unwrap_err();
    assert!(matches!(err, LightClientError::EmptyCommittee { .. }), "{err}");
}

#[test]
fn a_failed_advance_leaves_the_trusted_set_as_it_was() {
    let sets = sets();
    let trusted = anchored(&sets);
    let before = trusted.clone();
    let entry = checkpoint_entry(CheckpointSpec {
        next_set: ValidatorSetHash::from_array([7; 32]),
        ..CheckpointSpec::handover(&sets[1], &sets[2])
    });

    trusted.advance(&entry).unwrap_err();
    assert_eq!(trusted, before);
}

#[test]
fn the_walk_plan_reanchors_only_past_the_walk_limit() {
    let sets = sets();
    let trusted = anchored(&sets);

    assert_eq!(plan_walk(&trusted, Epoch(1), 10), WalkPlan::UpToDate);
    assert_eq!(plan_walk(&trusted, Epoch(2), 10), WalkPlan::Walk {
        from_epoch: Epoch(2),
        to_epoch: Epoch(2)
    });
    assert_eq!(plan_walk(&trusted, Epoch(11), 10), WalkPlan::Walk {
        from_epoch: Epoch(2),
        to_epoch: Epoch(11)
    });
    assert_eq!(plan_walk(&trusted, Epoch(12), 10), WalkPlan::Reanchor);
}
