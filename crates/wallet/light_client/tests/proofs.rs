//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

mod support;

use ootle_network::Network;
use support::*;
use tari_engine_types::{
    ProtocolVersion,
    SubstateVersion,
    confidential::ClaimedOutputTombstone,
    fees::FeeReceipt,
    substate::{Substate, SubstateDiff, SubstateId, SubstateValue, hash_substate},
    transaction_receipt::{DiffSummary, FinalizeOutcome, TransactionReceipt},
};
use tari_indexer_client::types::{ProvenTransactionReceipt, SubstateProof, SubstateProofAnchor};
use tari_ootle_common_types::{Epoch, ShardGroup, VersionedSubstateId, shard::Shard};
use tari_ootle_light_client::{
    LightClientError,
    TrustedValidatorSet,
    VerifiedCommittee,
    check_diff_matches_receipt,
    down_proof_up_anchor,
    verify_down_proof,
};
use tari_ootle_transaction::TransactionId;
use tari_state_tree::{
    SPARSE_MERKLE_PLACEHOLDER_HASH,
    ShardGroupRootTree,
    SpreadPrefixStateTree,
    StateTreePayload,
    SubstateDownProof,
    SubstateTreeChange,
    SubstateValueProof,
    TreeHash,
    memory_store::MemoryTreeStore,
};
use tari_template_lib_types::{ClaimedOutputTombstoneAddress, Hash32};

const EPOCH: Epoch = Epoch(2);

fn trusted_set() -> (ValidatorSetFixture, TrustedValidatorSet) {
    let set = ValidatorSetFixture::new(EPOCH.as_u64());
    let trusted =
        TrustedValidatorSet::anchor_pinned(NETWORK, walk_shard(), EPOCH, &set.hash(), &set.committee(low())).unwrap();
    (set, trusted)
}

fn shard_group_of(shard: Shard) -> ShardGroup {
    if low().contains(&shard) { low() } else { high() }
}

fn tombstone(seed: u8) -> (SubstateId, SubstateValue) {
    (
        SubstateId::ClaimedOutputTombstone(ClaimedOutputTombstoneAddress::from_bytes(&[seed; 32]).unwrap()),
        SubstateValue::ClaimedOutputTombstone(ClaimedOutputTombstone { value: u64::from(seed) }),
    )
}

fn receipt(diff_summary: DiffSummary) -> TransactionReceipt {
    TransactionReceipt {
        outcome: FinalizeOutcome::Commit,
        diff_summary,
        fee_withdrawals: Box::new([]),
        events: Box::new([]),
        fee_receipt: FeeReceipt::default(),
        epoch: EPOCH,
        intent_commitment: Hash32::from_array([3; 32]),
    }
}

/// A substate proof of `id` at version 0 holding `value`, anchored at a block of its shard group that `set`'s
/// committee signed.
fn substate_proof(set: &ValidatorSetFixture, id: &SubstateId, value: &SubstateValue) -> SubstateProof {
    let versioned_id = VersionedSubstateId::new(id.clone(), SubstateVersion::ZERO);
    let shard = versioned_id.to_shard(NUM_PRESHARDS);
    let shard_group = shard_group_of(shard);
    let value_hash = hash_substate(NETWORK, value, SubstateVersion::ZERO, EPOCH);

    let mut store = MemoryTreeStore::<StateTreePayload>::new();
    let shard_root = SpreadPrefixStateTree::new(&mut store)
        .put_substate_changes(None, 1, vec![SubstateTreeChange::Up {
            id: versioned_id.clone(),
            value_hash,
        }])
        .unwrap();
    let group_tree = group_tree(shard, shard_root, 1);
    let (_, _, leaf_proof) = SpreadPrefixStateTree::new(&mut store)
        .get_proof(1, &versioned_id)
        .unwrap();
    let value_proof = SubstateValueProof::new(shard_root, 1, group_tree.get_proof(shard).unwrap().1, leaf_proof);

    SubstateProof {
        anchor: SubstateProofAnchor {
            epoch: EPOCH,
            shard_group,
            height: 5,
            block_id: Default::default(),
            state_merkle_root: Default::default(),
        },
        commit_proof: block_commit_proof(
            NETWORK,
            EPOCH,
            shard_group,
            5,
            group_tree.root(),
            set.signers(shard_group),
        ),
        value_proof: tari_bor::serde_codec::to_vec(&value_proof).unwrap(),
        value_hash_epoch: EPOCH,
    }
}

fn group_tree(shard: Shard, shard_root: TreeHash, version: u64) -> ShardGroupRootTree {
    ShardGroupRootTree::build(ProtocolVersion::at(NETWORK, EPOCH), [
        (Shard::global(), SPARSE_MERKLE_PLACEHOLDER_HASH, 0),
        (shard, shard_root, version),
    ])
    .unwrap()
}

fn committee_for(trusted: &TrustedValidatorSet, set: &ValidatorSetFixture, proof: &SubstateProof) -> VerifiedCommittee {
    trusted
        .verify_committee(&set.committee(proof.anchor.shard_group))
        .unwrap()
}

#[test]
fn it_verifies_a_substate_against_its_shard_groups_committee() {
    let (set, trusted) = trusted_set();
    let (id, value) = tombstone(1);
    let proof = substate_proof(&set, &id, &value);
    let committee = committee_for(&trusted, &set, &proof);

    let tip = committee
        .verify_substate(&id, SubstateVersion::ZERO, Some(&value), &proof)
        .unwrap();
    assert_eq!(tip.epoch, EPOCH);
    assert_eq!(tip.height.as_u64(), 5);
}

#[test]
fn a_substate_with_another_value_is_rejected() {
    let (set, trusted) = trusted_set();
    let (id, value) = tombstone(1);
    let proof = substate_proof(&set, &id, &value);
    let committee = committee_for(&trusted, &set, &proof);

    let (_, other_value) = tombstone(2);
    let err = committee
        .verify_substate(&id, SubstateVersion::ZERO, Some(&other_value), &proof)
        .unwrap_err();
    assert!(matches!(err, LightClientError::InvalidSubstateProof(_)), "{err}");
}

#[test]
fn a_substate_proof_checked_against_another_shard_groups_committee_is_rejected() {
    let (set, trusted) = trusted_set();
    let (id, value) = tombstone(1);
    let proof = substate_proof(&set, &id, &value);
    let other_group = if proof.anchor.shard_group == low() {
        high()
    } else {
        low()
    };
    let committee = trusted.verify_committee(&set.committee(other_group)).unwrap();

    let err = committee
        .verify_substate(&id, SubstateVersion::ZERO, Some(&value), &proof)
        .unwrap_err();
    assert!(
        matches!(err, LightClientError::CommitProofShardGroupMismatch { .. }),
        "{err}"
    );
}

#[test]
fn a_commit_proof_of_another_network_is_rejected() {
    let (set, trusted) = trusted_set();
    let committee = trusted.walk_committee();
    let commit_proof = block_commit_proof(
        Network::Esmeralda,
        EPOCH,
        low(),
        5,
        SPARSE_MERKLE_PLACEHOLDER_HASH,
        &set.low,
    );

    let err = committee.verify_commit_proof(&commit_proof).unwrap_err();
    assert!(matches!(err, LightClientError::WrongNetwork { .. }), "{err}");
}

#[test]
fn a_commit_proof_of_another_epoch_is_rejected() {
    let (set, trusted) = trusted_set();
    let commit_proof = block_commit_proof(NETWORK, Epoch(3), low(), 5, SPARSE_MERKLE_PLACEHOLDER_HASH, &set.low);

    let err = trusted.walk_committee().verify_commit_proof(&commit_proof).unwrap_err();
    assert!(
        matches!(err, LightClientError::CommitProofEpochMismatch { .. }),
        "{err}"
    );
}

#[test]
fn a_commit_proof_signed_by_others_is_rejected() {
    let (_, trusted) = trusted_set();
    let impostors = Signers::new(0xee, 4);
    let commit_proof = block_commit_proof(NETWORK, EPOCH, low(), 5, SPARSE_MERKLE_PLACEHOLDER_HASH, &impostors);

    let err = trusted.walk_committee().verify_commit_proof(&commit_proof).unwrap_err();
    assert!(matches!(err, LightClientError::InvalidCommitProof(_)), "{err}");
}

#[test]
fn it_verifies_a_committed_receipt() {
    let (set, trusted) = trusted_set();
    let transaction_id = TransactionId::new([9; 32]);
    let receipt = receipt(DiffSummary::default());
    let receipt_id = SubstateId::TransactionReceipt(transaction_id.into_receipt_address());
    let proof = substate_proof(&set, &receipt_id, &SubstateValue::from(receipt.clone()));
    let committee = committee_for(&trusted, &set, &proof);

    let mut proven = ProvenTransactionReceipt {
        version: SubstateVersion::ZERO,
        receipt,
        proof: Some(proof),
    };
    committee.verify_receipt(transaction_id, &proven).unwrap();

    let err = committee
        .verify_receipt(TransactionId::new([8; 32]), &proven)
        .unwrap_err();
    assert!(
        matches!(err, LightClientError::InvalidSubstateProof(_)),
        "the receipt is another transaction's: {err}"
    );

    proven.proof = None;
    let err = committee.verify_receipt(transaction_id, &proven).unwrap_err();
    assert!(matches!(err, LightClientError::MissingReceiptProof { .. }), "{err}");
}

fn diff_of(ups: &[(SubstateId, SubstateValue)], downs: &[(SubstateId, u64)]) -> SubstateDiff {
    let mut diff = SubstateDiff::new();
    for (id, value) in ups {
        diff.up(id.clone(), Substate::new(SubstateVersion::ZERO, value.clone()));
    }
    for (id, version) in downs {
        diff.down(id.clone(), SubstateVersion::new(*version));
    }
    diff
}

#[test]
fn a_diff_matches_the_receipt_built_from_it() {
    let transaction_id = TransactionId::new([9; 32]);
    let diff = diff_of(&[tombstone(1), tombstone(2)], &[
        (tombstone(3).0, 4),
        (tombstone(1).0, 0),
    ]);
    let receipt = receipt(DiffSummary::from_diff(NETWORK, &diff, EPOCH));

    // The diff the engine returns also ups the receipt itself.
    let mut full_diff = diff.clone();
    full_diff.up(
        SubstateId::TransactionReceipt(transaction_id.into_receipt_address()),
        Substate::new(SubstateVersion::ZERO, receipt.clone()),
    );
    check_diff_matches_receipt(NETWORK, transaction_id, &receipt, &full_diff).unwrap();
}

#[test]
fn a_diff_that_differs_from_the_receipt_is_rejected() {
    let transaction_id = TransactionId::new([9; 32]);
    let diff = diff_of(&[tombstone(1)], &[(tombstone(3).0, 4)]);
    let receipt = receipt(DiffSummary::from_diff(NETWORK, &diff, EPOCH));

    let cases = [
        (
            "an extra up",
            diff_of(&[tombstone(1), tombstone(2)], &[(tombstone(3).0, 4)]),
        ),
        ("a missing up", diff_of(&[], &[(tombstone(3).0, 4)])),
        (
            "another value",
            diff_of(&[(tombstone(1).0, tombstone(2).1)], &[(tombstone(3).0, 4)]),
        ),
        ("a missing down", diff_of(&[tombstone(1)], &[])),
        ("another down version", diff_of(&[tombstone(1)], &[(tombstone(3).0, 5)])),
    ];
    for (case, diff) in cases {
        let err = check_diff_matches_receipt(NETWORK, transaction_id, &receipt, &diff).unwrap_err();
        assert!(
            matches!(err, LightClientError::DiffDoesNotMatchReceipt { .. }),
            "{case}: {err}"
        );
    }
}

/// A down proof of `id`, up at height 5 and absent at height 9 of its shard group's chain, both in [`EPOCH`].
fn down_proof(set: &ValidatorSetFixture, id: &SubstateId) -> (Vec<u8>, Vec<u8>) {
    let versioned_id = VersionedSubstateId::new(id.clone(), SubstateVersion::ZERO);
    let shard = versioned_id.to_shard(NUM_PRESHARDS);
    let shard_group = shard_group_of(shard);
    let signers = set.signers(shard_group);
    let value_hash = Hash32::from_array([5; 32]);

    let mut store = MemoryTreeStore::<StateTreePayload>::new();
    let up_root = SpreadPrefixStateTree::new(&mut store)
        .put_substate_changes(None, 1, vec![SubstateTreeChange::Up {
            id: versioned_id.clone(),
            value_hash,
        }])
        .unwrap();
    let down_root = SpreadPrefixStateTree::new(&mut store)
        .put_substate_changes(Some(1), 2, vec![SubstateTreeChange::Down {
            id: versioned_id.clone(),
        }])
        .unwrap();
    let up_tree = group_tree(shard, up_root, 1);
    let down_tree = group_tree(shard, down_root, 2);
    let (_, value, up_leaf) = SpreadPrefixStateTree::new(&mut store)
        .get_proof(1, &versioned_id)
        .unwrap();
    let (_, _, down_leaf) = SpreadPrefixStateTree::new(&mut store)
        .get_proof(2, &versioned_id)
        .unwrap();

    let proof = SubstateDownProof {
        up: SubstateValueProof::new(up_root, 1, up_tree.get_proof(shard).unwrap().1, up_leaf),
        up_value_hash: value.unwrap().0,
        up_commit_proof: block_commit_proof(NETWORK, EPOCH, shard_group, 5, up_tree.root(), signers),
        down: SubstateValueProof::new(down_root, 2, down_tree.get_proof(shard).unwrap().1, down_leaf),
    };
    (
        tari_bor::serde_codec::to_vec(&proof).unwrap(),
        block_commit_proof(NETWORK, EPOCH, shard_group, 9, down_tree.root(), signers),
    )
}

#[test]
fn it_verifies_a_down_proof_with_the_committees_of_both_anchors() {
    let (set, trusted) = trusted_set();
    let (id, _) = tombstone(1);
    let (proof, down_commit_proof) = down_proof(&set, &id);

    let (up_epoch, up_shard_group) = down_proof_up_anchor(&proof).unwrap();
    assert_eq!(up_epoch, EPOCH);
    let committee = trusted.verify_committee(&set.committee(up_shard_group)).unwrap();
    verify_down_proof(
        &id,
        SubstateVersion::ZERO,
        &proof,
        &committee,
        &committee,
        &down_commit_proof,
    )
    .unwrap();

    let impostors =
        ValidatorSetFixture::with_signers(NETWORK, EPOCH.as_u64(), Signers::new(0xe0, 4), Signers::new(0xe1, 4));
    let impostor_trust = TrustedValidatorSet::anchor_pinned(
        NETWORK,
        walk_shard(),
        EPOCH,
        &impostors.hash(),
        &impostors.committee(low()),
    )
    .unwrap();
    let other_committee = impostor_trust
        .verify_committee(&impostors.committee(up_shard_group))
        .unwrap();
    let err = verify_down_proof(
        &id,
        SubstateVersion::ZERO,
        &proof,
        &committee,
        &other_committee,
        &down_commit_proof,
    )
    .unwrap_err();
    assert!(matches!(err, LightClientError::InvalidCommitProof(_)), "{err}");
}
