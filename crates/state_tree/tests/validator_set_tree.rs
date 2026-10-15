//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use ootle_network::Network;
use tari_ootle_common_types::{
    Epoch,
    NumPreshards,
    ShardGroup,
    ValidatorSetError,
    ValidatorSetHash,
    ValidatorSetMember,
    VotePower,
    committee::{Committee, CommitteeMember},
};
use tari_state_tree::{
    SPARSE_MERKLE_PLACEHOLDER_HASH,
    SparseMerkleProofExt,
    TreeHash,
    ValidatorSetCommitteeProof,
    ValidatorSetProofError,
    ValidatorSetTree,
    ValidatorSetTreeError,
};
use tari_template_lib_types::{Hash32, crypto::RistrettoPublicKeyBytes};

/// The [`ValidatorSetHash`] of [`sample_tree`].
const SAMPLE_SET_HASH: &str = "bfdcf97b147fa328cda0fabb01a5b2c4863f450bd085c59c7f41b38d7b9b881e";
/// The committee root of [`sample_tree`].
const SAMPLE_COMMITTEE_ROOT: &str = "5c3ffb14ccdf1af74a643c0c7aa2c5c5a0f9de74b55c9b04e14bcec2cf49d81e";
/// The CBOR encoding of the Merkle proof for shard group 0-1 of [`sample_tree`].
const SAMPLE_PROOF_CBOR: &str = "8282582085e0fbe3f7469cf36076378d541a98ff75229484bceda9bbde30a821f5fb0be95820cdea39abface15bb1cf7dc2b56011a04aa220fb7acd6f21dd387bb22533f7c1e8182008258205207a5c228122830b01276579c669167430ce311fdd5448360324677db7a99be58202dfd535a9c0bedfa396db656d5533885a85f10ea4cfeded3494e12571c8e750c";

fn pk(byte: u8) -> RistrettoPublicKeyBytes {
    RistrettoPublicKeyBytes::from_bytes(&[byte; 32]).unwrap()
}

fn num_preshards() -> NumPreshards {
    NumPreshards::try_from(4u32).unwrap()
}

fn low() -> ShardGroup {
    ShardGroup::new(0u32, 1u32)
}

fn high() -> ShardGroup {
    ShardGroup::new(2u32, 3u32)
}

fn committee(members: &[(u8, u64)]) -> Committee<u8> {
    Committee::new(
        members
            .iter()
            .map(|(key_byte, power)| CommitteeMember {
                address: *key_byte,
                public_key: pk(*key_byte),
                vote_power: VotePower::of(*power),
            })
            .collect(),
    )
}

fn low_committee() -> Committee<u8> {
    committee(&[(1, 1), (2, 1)])
}

fn high_committee() -> Committee<u8> {
    committee(&[(3, 2), (4, 1)])
}

fn leaf(shard_group: ShardGroup, committee: &Committee<u8>) -> (ShardGroup, Hash32) {
    (
        shard_group,
        committee.calculate_validator_set_leaf_hash(&shard_group).unwrap(),
    )
}

fn sample_tree() -> ValidatorSetTree {
    ValidatorSetTree::build(Network::LocalNet, Epoch(7), num_preshards(), [
        leaf(low(), &low_committee()),
        leaf(high(), &high_committee()),
    ])
    .unwrap()
}

fn members(committee: &Committee<u8>) -> Vec<ValidatorSetMember> {
    committee
        .iter()
        .map(|member| ValidatorSetMember {
            public_key: member.public_key,
            vote_power: member.vote_power,
        })
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn from_hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

#[test]
fn the_sample_set_hashes_to_the_golden_vector() {
    let tree = sample_tree();
    assert_eq!(tree.committee_root().to_string(), SAMPLE_COMMITTEE_ROOT);
    assert_eq!(tree.hash().to_string(), SAMPLE_SET_HASH);
}

#[test]
fn the_set_hash_commits_to_the_committee_root() {
    let tree = sample_tree();
    assert_eq!(
        tree.hash(),
        ValidatorSetHash::calculate(
            Network::LocalNet,
            Epoch(7),
            num_preshards(),
            &Hash32::from_array(tree.committee_root().into_array())
        )
    );
}

#[test]
fn the_root_does_not_depend_on_leaf_order() {
    let reversed = ValidatorSetTree::build(Network::LocalNet, Epoch(7), num_preshards(), [
        leaf(high(), &high_committee()),
        leaf(low(), &low_committee()),
    ])
    .unwrap();
    assert_eq!(reversed.hash(), sample_tree().hash());
}

#[test]
fn the_golden_proof_verifies_against_the_golden_hash() {
    let proof = sample_tree().committee_proof(&low(), &low_committee()).unwrap();
    assert_eq!(hex(&minicbor::to_vec(&proof.proof).unwrap()), SAMPLE_PROOF_CBOR);

    let golden = ValidatorSetCommitteeProof {
        network: Network::LocalNet,
        epoch: Epoch(7),
        num_preshards: num_preshards(),
        committee_root: TreeHash::try_from_bytes(&from_hex(SAMPLE_COMMITTEE_ROOT)).unwrap(),
        shard_group: low(),
        members: members(&low_committee()),
        proof: minicbor::decode::<SparseMerkleProofExt>(&from_hex(SAMPLE_PROOF_CBOR)).unwrap(),
    };
    let expected = ValidatorSetHash::from_array(from_hex(SAMPLE_SET_HASH).try_into().unwrap());
    assert_eq!(golden.verify(&expected).unwrap(), members(&low_committee()).as_slice());
}

#[test]
fn every_committee_proves_against_the_set_hash() {
    let tree = sample_tree();
    for (shard_group, committee) in [(low(), low_committee()), (high(), high_committee())] {
        let proof = tree.committee_proof(&shard_group, &committee).unwrap();
        assert_eq!(proof.verify(&tree.hash()).unwrap(), members(&committee).as_slice());
    }
}

#[test]
fn a_proof_survives_a_json_round_trip() {
    let tree = sample_tree();
    let proof = tree.committee_proof(&high(), &high_committee()).unwrap();

    let decoded: ValidatorSetCommitteeProof = serde_json::from_str(&serde_json::to_string(&proof).unwrap()).unwrap();
    assert_eq!(decoded, proof);
    decoded.verify(&tree.hash()).unwrap();
}

#[test]
fn a_shard_group_outside_the_set_has_no_proof() {
    let outside = ShardGroup::new(0u32, 3u32);
    let err = sample_tree().committee_proof(&outside, &low_committee()).unwrap_err();
    assert!(
        matches!(err, ValidatorSetTreeError::ShardGroupNotInSet { shard_group, epoch }
            if shard_group == outside && epoch == Epoch(7)),
        "{err}"
    );
}

#[test]
fn a_committee_that_differs_from_its_leaf_has_no_proof() {
    let err = sample_tree()
        .committee_proof(&low(), &committee(&[(1, 1), (9, 1)]))
        .unwrap_err();
    assert!(
        matches!(err, ValidatorSetTreeError::CommitteeDoesNotMatchLeaf { shard_group, .. } if shard_group == low()),
        "{err}"
    );
}

#[test]
fn a_repeated_shard_group_is_rejected() {
    let Err(err) = ValidatorSetTree::build(Network::LocalNet, Epoch(7), num_preshards(), [
        leaf(low(), &low_committee()),
        leaf(low(), &high_committee()),
    ]) else {
        panic!("a repeated shard group must be rejected");
    };
    assert!(
        matches!(err, ValidatorSetTreeError::DuplicateShardGroup { shard_group, .. } if shard_group == low()),
        "{err}"
    );
}

#[test]
fn a_proof_fails_against_another_set_hash() {
    let proof = sample_tree().committee_proof(&low(), &low_committee()).unwrap();
    let other = ValidatorSetHash::from_array([1; 32]);
    assert!(matches!(
        proof.verify(&other),
        Err(ValidatorSetProofError::SetHashMismatch { .. })
    ));
}

#[test]
fn a_proof_with_a_changed_member_fails() {
    let tree = sample_tree();
    let forgeries = [
        committee(&[(1, 1), (9, 1)]),
        committee(&[(1, 1), (2, 5)]),
        committee(&[(1, 1)]),
        committee(&[(1, 1), (2, 1), (5, 1)]),
    ];
    for forged in &forgeries {
        let mut proof = tree.committee_proof(&low(), &low_committee()).unwrap();
        proof.members = members(forged);
        assert!(
            matches!(
                proof.verify(&tree.hash()),
                Err(ValidatorSetProofError::CommitteeNotInSet { .. })
            ),
            "a forged committee must not verify"
        );
    }
}

#[test]
fn a_proof_with_members_out_of_order_fails() {
    let tree = sample_tree();
    let mut proof = tree.committee_proof(&low(), &low_committee()).unwrap();
    proof.members.reverse();
    assert!(matches!(
        proof.verify(&tree.hash()),
        Err(ValidatorSetProofError::MalformedCommittee(
            ValidatorSetError::CommitteeNotSorted { .. }
        ))
    ));
}

#[test]
fn a_proof_relabelled_to_another_shard_group_fails() {
    let tree = sample_tree();

    let mut relabelled = tree.committee_proof(&low(), &low_committee()).unwrap();
    relabelled.shard_group = high();
    assert!(matches!(
        relabelled.verify(&tree.hash()),
        Err(ValidatorSetProofError::CommitteeNotInSet { .. })
    ));

    let mut swapped_members = tree.committee_proof(&high(), &high_committee()).unwrap();
    swapped_members.members = members(&low_committee());
    assert!(matches!(
        swapped_members.verify(&tree.hash()),
        Err(ValidatorSetProofError::CommitteeNotInSet { .. })
    ));

    let mut swapped_proof = tree.committee_proof(&high(), &high_committee()).unwrap();
    swapped_proof.proof = tree.committee_proof(&low(), &low_committee()).unwrap().proof;
    assert!(matches!(
        swapped_proof.verify(&tree.hash()),
        Err(ValidatorSetProofError::CommitteeNotInSet { .. })
    ));
}

#[test]
fn a_proof_with_a_changed_outer_field_fails() {
    let tree = sample_tree();
    let tampered: [fn(&mut ValidatorSetCommitteeProof); 4] = [
        |p| p.epoch = Epoch(8),
        |p| p.network = Network::Esmeralda,
        |p| p.num_preshards = NumPreshards::try_from(8u32).unwrap(),
        |p| p.committee_root = TreeHash::new([7; 32]),
    ];
    for tamper in tampered {
        let mut proof = tree.committee_proof(&low(), &low_committee()).unwrap();
        tamper(&mut proof);
        assert!(matches!(
            proof.verify(&tree.hash()),
            Err(ValidatorSetProofError::SetHashMismatch { .. })
        ));
    }
}

#[test]
fn an_empty_set_commits_to_the_placeholder_root() {
    let tree = ValidatorSetTree::build(Network::LocalNet, Epoch(7), num_preshards(), []).unwrap();
    assert_eq!(tree.committee_root(), SPARSE_MERKLE_PLACEHOLDER_HASH);
    assert!(tree.committee_proof(&low(), &low_committee()).is_err());
}
