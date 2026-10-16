//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! A wallet that verifies its indexer reads a substate only once the indexer proves it, against a validator set the
//! wallet trusts, at a block recent enough to show the substate's current state.

mod support;

use tari_engine_types::{
    confidential::ClaimedOutputTombstone,
    substate::{SubstateId, SubstateValue},
};
use tari_ootle_common_types::{Epoch, SubstateVersion, VersionedSubstateId};
use tari_ootle_light_client::test_fixtures::{
    CheckpointSpec,
    NETWORK,
    NUM_PRESHARDS,
    Signers,
    ValidatorSetFixture,
    checkpoint_entry,
    high,
    shard_group_of,
    substate_proof,
};
use tari_ootle_wallet_sdk::{IndexerTrust, apis::substate::SubstateApiError};
use tari_template_lib::types::ClaimedOutputTombstoneAddress;

use crate::support::{ProvingNetwork, TestWithNetwork};

const VERSION: SubstateVersion = SubstateVersion::new(3);

fn substate(seed: u8) -> (SubstateId, SubstateValue) {
    (
        SubstateId::ClaimedOutputTombstone(ClaimedOutputTombstoneAddress::from_bytes(&[seed; 32]).unwrap()),
        SubstateValue::ClaimedOutputTombstone(ClaimedOutputTombstone { value: u64::from(seed) }),
    )
}

fn tombstone_value(value: &SubstateValue) -> u64 {
    match value {
        SubstateValue::ClaimedOutputTombstone(tombstone) => tombstone.value,
        _ => panic!("not a tombstone"),
    }
}

/// The error a verified read of `id` fails with.
async fn read_error(test: &TestWithNetwork<ProvingNetwork>, id: &SubstateId) -> SubstateApiError {
    match test.sdk().substate_api().fetch_substate_from_network(id, None).await {
        Ok(_) => panic!("the read of {id} succeeded"),
        Err(err) => err,
    }
}

/// Validator sets of epochs 1 to 5.
fn sets() -> Vec<ValidatorSetFixture> {
    (1..=5).map(ValidatorSetFixture::new).collect()
}

/// A wallet that anchored on the set of epoch 2 and walked to the set of epoch 5.
async fn walked_wallet(
    indexer_trust: IndexerTrust,
) -> (
    Vec<ValidatorSetFixture>,
    ProvingNetwork,
    TestWithNetwork<ProvingNetwork>,
) {
    let sets = sets();
    let network = ProvingNetwork::new(&sets, 0);
    let test = TestWithNetwork::without_accounts_trusting(network.clone(), indexer_trust);
    test.sdk().validator_set_api().sync().await.unwrap();
    network.set_latest(3);
    let trusted = test.sdk().validator_set_api().sync().await.unwrap();
    assert_eq!(trusted.epoch(), Epoch(5));
    (sets, network, test)
}

#[tokio::test]
async fn a_substate_proven_in_the_latest_trusted_epoch_is_read() {
    let (sets, network, test) = walked_wallet(IndexerTrust::Verify).await;
    let (id, value) = substate(1);
    let proof = substate_proof(&sets[4], &id, VERSION, &value);
    network.serve_substate(id.clone(), VERSION, value.clone(), Some(proof));

    let read = test
        .sdk()
        .substate_api()
        .fetch_substate_from_network(&id, None)
        .await
        .unwrap();
    assert_eq!(read.id.version(), VERSION);
    assert_eq!(tombstone_value(&read.substate), tombstone_value(&value));

    let fetched = test
        .sdk()
        .substate_api()
        .get_substates_from_network(vec![id.clone()])
        .await
        .unwrap();
    assert_eq!(tombstone_value(fetched[&id].substate_value()), tombstone_value(&value));
}

#[tokio::test]
async fn a_substate_proven_an_epoch_before_the_latest_trusted_set_is_read() {
    let (sets, network, test) = walked_wallet(IndexerTrust::Verify).await;
    let (id, value) = substate(1);
    let proof = substate_proof(&sets[3], &id, VERSION, &value);
    network.serve_substate(id.clone(), VERSION, value, Some(proof));

    test.sdk()
        .substate_api()
        .fetch_substate_from_network(&id, None)
        .await
        .unwrap();
}

#[tokio::test]
async fn a_substate_proven_too_long_before_the_latest_trusted_set_is_refused() {
    let (sets, network, test) = walked_wallet(IndexerTrust::Verify).await;
    let (id, value) = substate(1);
    let proof = substate_proof(&sets[2], &id, VERSION, &value);
    network.serve_substate(id.clone(), VERSION, value, Some(proof));

    let err = read_error(&test, &id).await;
    assert!(
        matches!(err, SubstateApiError::SubstateProofStale {
            anchor_epoch: Epoch(3),
            latest_trusted_epoch: Epoch(5),
            ..
        }),
        "{err}"
    );
}

#[tokio::test]
async fn a_substate_served_without_a_proof_is_refused() {
    let (_, network, test) = walked_wallet(IndexerTrust::Verify).await;
    let (id, value) = substate(1);
    network.serve_substate(id.clone(), VERSION, value, None);

    let err = read_error(&test, &id).await;
    assert!(matches!(err, SubstateApiError::SubstateUnproven { .. }), "{err}");
}

#[tokio::test]
async fn a_substate_whose_proof_shows_another_value_is_refused() {
    let (sets, network, test) = walked_wallet(IndexerTrust::Verify).await;
    let (id, value) = substate(1);
    let (_, proven_value) = substate(2);
    let proof = substate_proof(&sets[4], &id, VERSION, &proven_value);
    network.serve_substate(id.clone(), VERSION, value, Some(proof));

    let err = read_error(&test, &id).await;
    assert!(matches!(err, SubstateApiError::SubstateUnverified { .. }), "{err}");
}

#[tokio::test]
async fn a_substate_proven_before_the_anchor_is_refused() {
    let sets = sets();
    // The wallet anchors on the set of epoch 4, which the latest checkpoint names.
    let network = ProvingNetwork::new(&sets, 2);
    let test = TestWithNetwork::without_accounts_trusting(network.clone(), IndexerTrust::Verify);
    let (id, value) = substate(1);
    let proof = substate_proof(&sets[2], &id, VERSION, &value);
    network.serve_substate(id.clone(), VERSION, value, Some(proof));

    let err = read_error(&test, &id).await;
    assert!(
        matches!(err, SubstateApiError::SubstateProvenOutsideTrustedSets {
            epoch: Epoch(3),
            ..
        }),
        "{err}"
    );
}

#[tokio::test]
async fn a_fetch_with_any_unproven_substate_is_refused() {
    let (sets, network, test) = walked_wallet(IndexerTrust::Verify).await;
    let (proven_id, proven_value) = substate(1);
    let proof = substate_proof(&sets[4], &proven_id, VERSION, &proven_value);
    network.serve_substate(proven_id.clone(), VERSION, proven_value, Some(proof));
    let (unproven_id, unproven_value) = substate(2);
    network.serve_substate(unproven_id.clone(), VERSION, unproven_value, None);

    let err = test
        .sdk()
        .substate_api()
        .get_substates_from_network(vec![proven_id, unproven_id.clone()])
        .await
        .unwrap_err();
    assert!(
        matches!(&err, SubstateApiError::SubstateUnproven { substate_id } if *substate_id == unproven_id),
        "{err}"
    );
}

#[tokio::test]
async fn a_trusting_wallet_reads_without_proofs() {
    let (_, network, test) = walked_wallet(IndexerTrust::Trust).await;
    let (id, value) = substate(1);
    network.serve_substate(id.clone(), VERSION, value.clone(), None);

    let read = test
        .sdk()
        .substate_api()
        .fetch_substate_from_network(&id, None)
        .await
        .unwrap();
    assert_eq!(tombstone_value(&read.substate), tombstone_value(&value));
}

/// `n` substates of the high shard group. The wallet walks the low one, so a committee of the high one is fetched.
fn high_substates(n: usize) -> Vec<(SubstateId, SubstateValue)> {
    let substates = (1..=u8::MAX)
        .map(substate)
        .filter(|(id, _)| {
            shard_group_of(VersionedSubstateId::new(id.clone(), VERSION).to_shard(NUM_PRESHARDS)) == high()
        })
        .take(n)
        .collect::<Vec<_>>();
    assert_eq!(substates.len(), n);
    substates
}

#[tokio::test]
async fn a_committee_is_fetched_once_for_the_reads_it_proves() {
    let (sets, network, test) = walked_wallet(IndexerTrust::Verify).await;
    let substates = high_substates(2);
    for (id, value) in &substates {
        let proof = substate_proof(&sets[4], id, VERSION, value);
        network.serve_substate(id.clone(), VERSION, value.clone(), Some(proof));
    }

    let before = network.committee_requests();
    for (id, _) in &substates {
        test.sdk()
            .substate_api()
            .fetch_substate_from_network(id, None)
            .await
            .unwrap();
    }
    assert_eq!(network.committee_requests() - before, 1);
}

#[tokio::test]
async fn a_reanchor_verifies_reads_with_the_new_sets_committees() {
    let (sets, network, test) = walked_wallet(IndexerTrust::Verify).await;
    let [(old_id, old_value), (new_id, new_value)] = high_substates(2).try_into().unwrap();
    let old_proof = substate_proof(&sets[4], &old_id, VERSION, &old_value);
    network.serve_substate(old_id.clone(), VERSION, old_value, Some(old_proof));
    test.sdk()
        .substate_api()
        .fetch_substate_from_network(&old_id, None)
        .await
        .unwrap();

    // The indexer now serves a different validator set for epoch 5, which the wallet re-anchors on.
    let other = ValidatorSetFixture::with_signers(NETWORK, 5, Signers::new(0xa0, 4), Signers::new(0xa1, 4));
    network.replace_checkpoint(3, checkpoint_entry(CheckpointSpec::handover(&sets[3], &other)));
    network.replace_committees(&other);
    let trusted = test.sdk().validator_set_api().reanchor().await.unwrap();
    assert_eq!(trusted.validator_set_hash(), &other.hash());

    let err = read_error(&test, &old_id).await;
    assert!(matches!(err, SubstateApiError::SubstateUnverified { .. }), "{err}");
    let new_proof = substate_proof(&other, &new_id, VERSION, &new_value);
    network.serve_substate(new_id.clone(), VERSION, new_value, Some(new_proof));
    test.sdk()
        .substate_api()
        .fetch_substate_from_network(&new_id, None)
        .await
        .unwrap();
}

#[tokio::test]
async fn a_substate_older_than_the_version_asked_for_is_refused() {
    let (sets, network, test) = walked_wallet(IndexerTrust::Verify).await;
    let (id, value) = substate(1);
    let proof = substate_proof(&sets[4], &id, VERSION, &value);
    network.serve_substate(id.clone(), VERSION, value, Some(proof));

    let err = match test
        .sdk()
        .substate_api()
        .fetch_substate_from_network(&id, Some(VERSION.next()))
        .await
    {
        Ok(_) => panic!("an older version was read"),
        Err(err) => err,
    };
    assert!(
        matches!(err, SubstateApiError::SubstateOlderThanRequested { .. }),
        "{err}"
    );
}
