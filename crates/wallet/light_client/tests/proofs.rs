//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use ootle_network::Network;
use tari_engine_types::{
    SubstateVersion,
    commit_result::{ExecutionFailureCode, FinalizeResult, RejectReason, TransactionResult},
    confidential::ClaimedOutputTombstone,
    events::Event,
    fees::FeeReceipt,
    non_fungible::NonFungibleContainer,
    resource::Resource,
    substate::{Substate, SubstateDiff, SubstateId, SubstateValue, hash_substate},
    transaction_receipt::{DiffSummary, FinalizeOutcome, TransactionReceipt},
};
use tari_indexer_client::types::{ProvenTransactionReceipt, SubstateProof};
use tari_ootle_common_types::Epoch;
use tari_ootle_light_client::{
    LightClientError,
    TrustedValidatorSet,
    VerifiedCommittee,
    check_diff_matches_receipt,
    check_result_matches_receipt,
    down_proof_up_anchor,
    test_fixtures::*,
    verify_down_proof,
};
use tari_ootle_transaction::TransactionId;
use tari_state_tree::SPARSE_MERKLE_PLACEHOLDER_HASH;
use tari_template_lib_types::{
    ClaimedOutputTombstoneAddress,
    Hash32,
    Metadata,
    NonFungibleAddress,
    NonFungibleId,
    ResourceAddress,
    ResourceType,
    SubstateOwnerRule,
};

const EPOCH: Epoch = Epoch(2);

fn trusted_set() -> (ValidatorSetFixture, TrustedValidatorSet) {
    let set = ValidatorSetFixture::new(EPOCH.as_u64());
    let trusted =
        TrustedValidatorSet::anchor_pinned(NETWORK, walk_shard(), EPOCH, &set.hash(), &set.committee(low())).unwrap();
    (set, trusted)
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

fn committee_for(trusted: &TrustedValidatorSet, set: &ValidatorSetFixture, proof: &SubstateProof) -> VerifiedCommittee {
    trusted
        .verify_committee(&set.committee(proof.anchor.shard_group))
        .unwrap()
}

#[test]
fn it_verifies_a_substate_against_its_shard_groups_committee() {
    let (set, trusted) = trusted_set();
    let (id, value) = tombstone(1);
    let proof = substate_proof(&set, &id, SubstateVersion::ZERO, &value);
    let committee = committee_for(&trusted, &set, &proof);

    let (tip, verified) = committee.verify_substate(&id, SubstateVersion::ZERO, &proof).unwrap();
    assert_eq!(tip.epoch, EPOCH);
    assert_eq!(tip.height.as_u64(), 5);
    assert_eq!(tari_bor::encode(&verified).unwrap(), tari_bor::encode(&value).unwrap());
}

#[test]
fn a_substate_with_another_value_is_rejected() {
    let (set, trusted) = trusted_set();
    let (id, value) = tombstone(1);
    let mut proof = substate_proof(&set, &id, SubstateVersion::ZERO, &value);
    let committee = committee_for(&trusted, &set, &proof);

    let (_, other_value) = tombstone(2);
    proof.value = Some(tari_bor::encode(&other_value).unwrap());
    let err = committee
        .verify_substate(&id, SubstateVersion::ZERO, &proof)
        .unwrap_err();
    assert!(matches!(err, LightClientError::InvalidSubstateProof(_)), "{err}");
}

#[test]
fn a_substate_proof_without_its_value_is_rejected() {
    let (set, trusted) = trusted_set();
    let (id, value) = tombstone(1);
    let mut proof = substate_proof(&set, &id, SubstateVersion::ZERO, &value);
    let committee = committee_for(&trusted, &set, &proof);

    proof.value = None;
    let err = committee
        .verify_substate(&id, SubstateVersion::ZERO, &proof)
        .unwrap_err();
    assert!(matches!(err, LightClientError::MissingSubstateValue { .. }), "{err}");

    proof.value = Some(vec![0xff]);
    let err = committee
        .verify_substate(&id, SubstateVersion::ZERO, &proof)
        .unwrap_err();
    assert!(matches!(err, LightClientError::Decode { .. }), "{err}");
}

#[test]
fn a_substate_proof_checked_against_another_shard_groups_committee_is_rejected() {
    let (set, trusted) = trusted_set();
    let (id, value) = tombstone(1);
    let proof = substate_proof(&set, &id, SubstateVersion::ZERO, &value);
    let other_group = if proof.anchor.shard_group == low() {
        high()
    } else {
        low()
    };
    let committee = trusted.verify_committee(&set.committee(other_group)).unwrap();

    let err = committee
        .verify_substate(&id, SubstateVersion::ZERO, &proof)
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
    let proof = substate_proof(
        &set,
        &receipt_id,
        SubstateVersion::ZERO,
        &SubstateValue::from(receipt.clone()),
    );
    let committee = committee_for(&trusted, &set, &proof);

    let mut proven = ProvenTransactionReceipt {
        version: SubstateVersion::ZERO,
        receipt,
        proof: Some(proof),
        result: None,
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

/// A committed transaction's diff, the receipt it committed and its result, with an event and fees in both.
fn committed(transaction_id: TransactionId) -> (SubstateDiff, TransactionReceipt, FinalizeResult) {
    let diff = diff_of(&[tombstone(1)], &[(tombstone(3).0, 4)]);
    let mut receipt = receipt(DiffSummary::from_diff(NETWORK, &diff, EPOCH));
    receipt.events = Box::new([event("std.vault.deposit")]);
    receipt.fee_receipt = FeeReceipt::builder()
        .with_total_fee_payment(800)
        .with_total_fees_paid(500)
        .build();
    let result = FinalizeResult::new(
        Hash32::from_array(transaction_id.into_array()),
        vec![],
        receipt.events.to_vec(),
        TransactionResult::Accept(diff.clone()),
        receipt.fee_receipt.clone(),
    );
    (diff, receipt, result)
}

fn event(topic: &str) -> Event {
    Event::new(
        None,
        Hash32::from_array([3; 32]),
        topic.to_string(),
        Metadata::default(),
    )
}

fn rejection() -> RejectReason {
    RejectReason::ExecutionFailure {
        code: ExecutionFailureCode::TemplateError,
        message: "failed".to_string(),
    }
}

#[test]
fn a_result_matches_the_receipt_committed_with_it() {
    let transaction_id = TransactionId::new([9; 32]);
    let (diff, mut receipt, mut result) = committed(transaction_id);
    check_result_matches_receipt(NETWORK, transaction_id, &receipt, &result).unwrap();

    receipt.outcome = FinalizeOutcome::FeeIntentCommit;
    result.result = TransactionResult::AcceptFeeRejectRest(diff, rejection());
    check_result_matches_receipt(NETWORK, transaction_id, &receipt, &result).unwrap();
}

#[test]
fn a_result_that_differs_from_the_receipt_is_rejected() {
    let transaction_id = TransactionId::new([9; 32]);
    let (diff, receipt, result) = committed(transaction_id);

    type Change = fn(&mut FinalizeResult, &SubstateDiff);
    let cases: [(&str, Change); 6] = [
        ("another transaction's", |result, _| {
            result.transaction_hash = Hash32::from_array([8; 32])
        }),
        ("a rejection", |result, _| {
            result.result = TransactionResult::Reject(rejection())
        }),
        ("only the fee intent committing", |result, diff| {
            result.result = TransactionResult::AcceptFeeRejectRest(diff.clone(), rejection())
        }),
        ("other events", |result, _| {
            result.events = vec![event("std.vault.withdraw")]
        }),
        ("other fees", |result, _| {
            result.fee_receipt = FeeReceipt::builder().with_total_fees_paid(1).build()
        }),
        ("another diff", |result, _| {
            result.result = TransactionResult::Accept(diff_of(&[tombstone(2)], &[(tombstone(3).0, 4)]))
        }),
    ];
    for (case, change) in cases {
        let mut changed = result.clone();
        change(&mut changed, &diff);
        let err = check_result_matches_receipt(NETWORK, transaction_id, &receipt, &changed).unwrap_err();
        assert!(
            matches!(
                err,
                LightClientError::ResultDoesNotMatchReceipt { .. } | LightClientError::DiffDoesNotMatchReceipt { .. }
            ),
            "{case}: {err}"
        );
    }
}

/// A down proof of `id`, up at height 5 of its shard group's chain in `up_set`'s epoch and absent at height 9 in
/// `down_set`'s, each signed by its set's committee.
fn down_proof(up_set: &ValidatorSetFixture, down_set: &ValidatorSetFixture, id: &SubstateId) -> (Vec<u8>, Vec<u8>) {
    let proof = substate_down_proof(up_set, down_set, id, SubstateVersion::ZERO);
    (proof.down_proof.unwrap(), proof.commit_proof)
}

#[test]
fn it_verifies_a_down_proof_with_the_committees_of_both_anchors() {
    let (set, trusted) = trusted_set();
    let (id, _) = tombstone(1);
    let (proof, down_commit_proof) = down_proof(&set, &set, &id);

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

#[test]
fn it_verifies_a_down_proof_across_a_handover_with_the_set_kept_for_each_epoch() {
    let (up_set, kept) = trusted_set();
    let down_set = ValidatorSetFixture::new(EPOCH.as_u64() + 1);
    let latest = kept
        .advance(&checkpoint_entry(CheckpointSpec::handover(&up_set, &down_set)))
        .unwrap();
    let (id, _) = tombstone(1);
    let (proof, down_commit_proof) = down_proof(&up_set, &down_set, &id);

    let (up_epoch, up_shard_group) = down_proof_up_anchor(&proof).unwrap();
    assert_eq!(up_epoch, EPOCH);
    let err = latest.verify_committee(&up_set.committee(up_shard_group)).unwrap_err();
    assert!(
        matches!(err, LightClientError::CommitteeEpochMismatch { .. }),
        "only the set of the up anchor's epoch verifies its committee: {err}"
    );

    let up_committee = kept.verify_committee(&up_set.committee(up_shard_group)).unwrap();
    let down_committee = latest.verify_committee(&down_set.committee(up_shard_group)).unwrap();
    verify_down_proof(
        &id,
        SubstateVersion::ZERO,
        &proof,
        &up_committee,
        &down_committee,
        &down_commit_proof,
    )
    .unwrap();
}

fn resource_address() -> ResourceAddress {
    ResourceAddress::from_hex(&"09".repeat(32)).unwrap()
}

fn nft_with_data(data: tari_bor::Value) -> (SubstateId, SubstateValue) {
    (
        SubstateId::NonFungible(NonFungibleAddress::new(resource_address(), NonFungibleId::from_u64(1))),
        SubstateValue::NonFungible(NonFungibleContainer::new(data, tari_bor::Value::Null)),
    )
}

fn resource_with_metadata_bytes(bytes: &[u8]) -> (SubstateId, SubstateValue) {
    let mut metadata = Metadata::new();
    metadata.insert_raw("k", tari_bor::decode_exact(bytes).unwrap());
    let resource = Resource::new(
        ResourceType::Fungible,
        SubstateOwnerRule::None,
        Default::default(),
        metadata,
        None,
        None,
        6,
        true,
    );
    (
        SubstateId::Resource(resource_address()),
        SubstateValue::Resource(Box::new(resource)),
    )
}

/// Committed values whose JSON form does not decode back to the same value: non-finite floats, and metadata whose CBOR
/// is valid but not canonical.
fn values_json_does_not_carry_exactly() -> Vec<(SubstateId, SubstateValue)> {
    vec![
        nft_with_data(tari_bor::Value::Float(f64::NAN)),
        nft_with_data(tari_bor::Value::Float(f64::INFINITY)),
        nft_with_data(tari_bor::Value::Float(f64::NEG_INFINITY)),
        // A non-minimal integer, an f32 and an indefinite-length text string.
        resource_with_metadata_bytes(&[0x18, 0x05]),
        resource_with_metadata_bytes(&[0xfa, 0x3f, 0xc0, 0x00, 0x00]),
        resource_with_metadata_bytes(&[0x7f, 0x61, 0x61, 0xff]),
    ]
}

#[test]
fn a_proof_served_as_json_verifies_and_yields_the_committed_value() {
    let (set, trusted) = trusted_set();
    for (id, value) in values_json_does_not_carry_exactly() {
        let proof = substate_proof(&set, &id, SubstateVersion::ZERO, &value);
        let committee = committee_for(&trusted, &set, &proof);
        let served: SubstateProof = serde_json::from_str(&serde_json::to_string(&proof).unwrap()).unwrap();

        let (_, verified) = committee
            .verify_substate(&id, SubstateVersion::ZERO, &served)
            .unwrap_or_else(|e| panic!("{id}: {e}"));
        assert_eq!(
            hash_substate(NETWORK, &verified, SubstateVersion::ZERO, EPOCH),
            hash_substate(NETWORK, &value, SubstateVersion::ZERO, EPOCH),
            "{id}"
        );
    }
}

/// A committed transaction whose diff ups each value [`values_json_does_not_carry_exactly`] lists and whose event
/// carries metadata whose CBOR is not canonical, with the receipt it committed and its result.
fn committed_with_values_json_does_not_carry_exactly(
    transaction_id: TransactionId,
) -> (TransactionReceipt, FinalizeResult) {
    let mut diff = SubstateDiff::new();
    for (seed, (id, value)) in values_json_does_not_carry_exactly().into_iter().enumerate() {
        let id = match id {
            SubstateId::NonFungible(address) => SubstateId::NonFungible(NonFungibleAddress::new(
                *address.resource_address(),
                NonFungibleId::from_u64(seed as u64),
            )),
            SubstateId::Resource(_) => {
                SubstateId::Resource(ResourceAddress::from_hex(&format!("{seed:02x}").repeat(32)).unwrap())
            },
            id => id,
        };
        diff.up(id, Substate::new(SubstateVersion::ZERO, value));
    }
    let mut receipt = receipt(DiffSummary::from_diff(NETWORK, &diff, EPOCH));
    let mut metadata = Metadata::new();
    metadata.insert_raw("k", tari_bor::decode_exact(&[0x18, 0x05]).unwrap());
    receipt.events = Box::new([Event::new(
        None,
        Hash32::from_array([3; 32]),
        "topic".to_string(),
        metadata,
    )]);
    let result = FinalizeResult::new(
        Hash32::from_array(transaction_id.into_array()),
        vec![],
        receipt.events.to_vec(),
        TransactionResult::Accept(diff),
        receipt.fee_receipt.clone(),
    );
    (receipt, result)
}

#[test]
fn a_result_served_as_json_verifies_and_yields_the_committed_result() {
    let (set, trusted) = trusted_set();
    let transaction_id = TransactionId::new([9; 32]);
    let (receipt, result) = committed_with_values_json_does_not_carry_exactly(transaction_id);
    let receipt_id = SubstateId::TransactionReceipt(transaction_id.into_receipt_address());
    let proof = substate_proof(
        &set,
        &receipt_id,
        SubstateVersion::ZERO,
        &SubstateValue::from(receipt.clone()),
    );
    let committee = committee_for(&trusted, &set, &proof);
    let proven = ProvenTransactionReceipt {
        version: SubstateVersion::ZERO,
        receipt,
        proof: Some(proof),
        result: Some(tari_bor::encode(&result).unwrap()),
    };
    let mut served: ProvenTransactionReceipt = serde_json::from_str(&serde_json::to_string(&proven).unwrap()).unwrap();

    let (_, verified) = committee.verify_result(transaction_id, &served).unwrap();
    assert_eq!(tari_bor::encode(&verified).unwrap(), tari_bor::encode(&result).unwrap());

    served.result = None;
    let err = committee.verify_result(transaction_id, &served).unwrap_err();
    assert!(
        matches!(err, LightClientError::MissingTransactionResult { .. }),
        "{err}"
    );
}
