//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

pub mod helpers;

use std::collections::HashSet;

use helpers::{PROOF_TEST_TREE_VERSION, build_substate_record, commit_substates, create_rocksdb, num_preshards};
use tari_common_types::types::FixedHash;
use tari_engine_types::ProtocolVersion;
use tari_ootle_common_types::{ShardGroup, SubstateVersion, VersionedSubstateId, shard::Shard};
use tari_ootle_storage::{
    ShardScopedTreeStoreReader,
    StateStore,
    StateStoreReadTransaction,
    SubstateProofGenerator,
    consensus_models::{EndOfEpochCommand, EpochCheckpoint, SubstateRecord, TreeRootSummary},
};
use tari_sidechain::{CommandCommitProof, SidechainBlockCommitProof, SidechainBlockHeader};
use tari_state_tree::{
    JmtHashScheme,
    SPARSE_MERKLE_PLACEHOLDER_HASH,
    SpreadPrefixStateTree,
    TreeHash,
    Version,
    compute_proof_for_hashes,
    compute_shard_group_root,
};

use crate::helpers::substate_id_seed;

const PROTOCOL_VERSION: ProtocolVersion = ProtocolVersion::V2;

/// Each shard's committed root and state version, global shard included, with a shard that has no
/// state tree at the empty root and version 0.
fn shard_states(tx: &impl StateStoreReadTransaction, shard_group: ShardGroup) -> Vec<(Shard, TreeHash, Version)> {
    shard_group
        .shard_iter_with_global()
        .map(|shard| {
            let Some(version) = tx.state_tree_versions_get_latest(shard).unwrap() else {
                return (shard, SPARSE_MERKLE_PLACEHOLDER_HASH, 0);
            };
            let mut store = ShardScopedTreeStoreReader::new(tx, shard);
            let root = SpreadPrefixStateTree::new(&mut store).get_root_hash(version).unwrap();
            (shard, root, version)
        })
        .collect()
}

/// The shard-group state merkle root a block header commits under `protocol_version`.
fn shard_group_root_at(
    tx: &impl StateStoreReadTransaction,
    shard_group: ShardGroup,
    protocol_version: ProtocolVersion,
) -> TreeHash {
    compute_shard_group_root(protocol_version, shard_states(tx, shard_group)).unwrap()
}

fn shard_group_root(tx: &impl StateStoreReadTransaction, shard_group: ShardGroup) -> TreeHash {
    shard_group_root_at(tx, shard_group, PROTOCOL_VERSION)
}

/// An end-of-epoch checkpoint of the store's current state, from a block produced under
/// `protocol_version`. Its commit proof is not signed, so it serves root computation only.
fn checkpoint_of(
    tx: &impl StateStoreReadTransaction,
    shard_group: ShardGroup,
    protocol_version: ProtocolVersion,
) -> EpochCheckpoint {
    let shard_states = shard_states(tx, shard_group);
    let state_merkle_root = compute_shard_group_root(protocol_version, shard_states.clone()).unwrap();
    let header = SidechainBlockHeader {
        network: 0,
        protocol_version: protocol_version.as_u32(),
        parent_id: Default::default(),
        justify_id: Default::default(),
        height: 0,
        epoch: 0,
        epoch_hash: Default::default(),
        shard_group: tari_sidechain::ShardGroup {
            start: shard_group.start().as_u32(),
            end_inclusive: shard_group.end().as_u32(),
        },
        proposed_by: Default::default(),
        state_merkle_root: FixedHash::new(state_merkle_root.into_array()),
        command_merkle_root: Default::default(),
        transaction_merkle_root: None,
        signature: Default::default(),
        accumulated_data: Default::default(),
        metadata_hash: Default::default(),
    };
    let command_hash = TreeHash::new([1; 32]);
    let (_, inclusion_proof) = compute_proof_for_hashes([command_hash].into_iter(), command_hash).unwrap();
    let summary = shard_states
        .into_iter()
        .map(|(shard, root_hash, state_version)| {
            (shard, TreeRootSummary {
                root_hash,
                state_version,
            })
        })
        .collect();
    EpochCheckpoint::new(
        CommandCommitProof::new(
            EndOfEpochCommand::new(FixedHash::default()),
            SidechainBlockCommitProof {
                header,
                proof_elements: vec![],
            },
            inclusion_proof,
        ),
        summary,
    )
}

/// The leaf value hash a verifier re-derives from the substate value to bind it to the committed leaf.
fn value_hash(substate: &SubstateRecord) -> TreeHash {
    TreeHash::new((*substate.state_hash()).into_array())
}

/// Substates whose ids land in at least `min_shards` distinct shards, so that a batch over them
/// exercises both a fresh and a reused shard-root proof.
fn substates_spanning_shards(count: u32, min_shards: usize) -> Vec<SubstateRecord> {
    // A substate's shard is read off the leading byte of its entity id, and `substate_id_seed` writes
    // the seed there big-endian, so the seed has to vary in its top byte to move between shards.
    let substates = (0..count)
        .map(|seed| {
            build_substate_record(
                &substate_id_seed(seed << 24),
                SubstateVersion::ZERO,
                PROOF_TEST_TREE_VERSION,
            )
        })
        .collect::<Vec<_>>();
    let shards = substates.iter().map(|s| s.created().in_shard).collect::<HashSet<_>>();
    assert!(
        shards.len() >= min_shards,
        "{count} substates landed in {} shard(s), need {min_shards}",
        shards.len()
    );
    substates
}

#[test]
fn proofs_for_a_batch_verify_against_one_shard_group_root() {
    let (db, _tmp) = create_rocksdb();
    let shard_group = ShardGroup::all_shards(num_preshards());
    let substates = substates_spanning_shards(8, 2);
    commit_substates(&db, &substates);

    let tx = db.create_read_tx().unwrap();
    let group_root = shard_group_root(&tx, shard_group);
    let mut generator = SubstateProofGenerator::new(&tx, shard_group, num_preshards(), PROTOCOL_VERSION).unwrap();

    for substate in &substates {
        let versioned_id = substate.to_versioned_substate_id();
        let proof = generator.generate(&versioned_id).unwrap().expect("shard has state");
        proof
            .verify_inclusion(
                JmtHashScheme::V1,
                PROTOCOL_VERSION,
                &group_root,
                num_preshards(),
                &versioned_id,
                &value_hash(substate),
            )
            .unwrap_or_else(|e| panic!("{versioned_id} in {}: {e}", substate.created().in_shard));
    }
}

/// The level-2 proof is cached per shard, so a batch that revisits a shard must not be served the
/// proof of a different shard's root.
#[test]
fn a_reused_shard_root_proof_belongs_to_its_own_shard() {
    let (db, _tmp) = create_rocksdb();
    let shard_group = ShardGroup::all_shards(num_preshards());
    let substates = substates_spanning_shards(8, 2);
    commit_substates(&db, &substates);

    let tx = db.create_read_tx().unwrap();
    let group_root = shard_group_root(&tx, shard_group);
    let mut generator = SubstateProofGenerator::new(&tx, shard_group, num_preshards(), PROTOCOL_VERSION).unwrap();

    // Fill the cache for every shard the substates live in, so that the second pass is served
    // entirely from it, then check each cached proof still verifies for its own substate.
    for substate in &substates {
        generator.generate(&substate.to_versioned_substate_id()).unwrap();
    }
    for substate in &substates {
        let versioned_id = substate.to_versioned_substate_id();
        let proof = generator.generate(&versioned_id).unwrap().expect("shard has state");
        proof
            .verify_inclusion(
                JmtHashScheme::V1,
                PROTOCOL_VERSION,
                &group_root,
                num_preshards(),
                &versioned_id,
                &value_hash(substate),
            )
            .unwrap_or_else(|e| panic!("{versioned_id} in {}: {e}", substate.created().in_shard));
    }
}

/// A version that is not up gets an exclusion proof - the shape a down substate in a batch is
/// answered with.
#[test]
fn a_version_that_is_not_up_gets_an_exclusion_proof() {
    let (db, _tmp) = create_rocksdb();
    let shard_group = ShardGroup::all_shards(num_preshards());
    let substates = substates_spanning_shards(4, 2);
    commit_substates(&db, &substates);

    let tx = db.create_read_tx().unwrap();
    let group_root = shard_group_root(&tx, shard_group);
    let mut generator = SubstateProofGenerator::new(&tx, shard_group, num_preshards(), PROTOCOL_VERSION).unwrap();

    for substate in &substates {
        let next_version = VersionedSubstateId::new(substate.substate_id().clone(), substate.version().next());
        let proof = generator.generate(&next_version).unwrap().expect("shard has state");
        proof
            .verify_exclusion(
                JmtHashScheme::V1,
                PROTOCOL_VERSION,
                &group_root,
                num_preshards(),
                &next_version,
            )
            .unwrap();
        proof
            .verify_inclusion(
                JmtHashScheme::V1,
                PROTOCOL_VERSION,
                &group_root,
                num_preshards(),
                &next_version,
                &value_hash(substate),
            )
            .unwrap_err();
    }
}

/// An exclusion proof is rooted at one shard, and a substate in any other shard is absent from that
/// shard's tree, so the proof must not verify as the absence of a substate that lives elsewhere.
#[test]
fn an_exclusion_proof_from_another_shard_does_not_prove_absence() {
    let (db, _tmp) = create_rocksdb();
    let shard_group = ShardGroup::all_shards(num_preshards());
    let substates = substates_spanning_shards(4, 2);
    commit_substates(&db, &substates);
    let present = &substates[0];
    let elsewhere = substates
        .iter()
        .find(|s| s.created().in_shard != present.created().in_shard)
        .expect("substates span more than one shard");

    let tx = db.create_read_tx().unwrap();
    let group_root = shard_group_root(&tx, shard_group);
    let mut generator = SubstateProofGenerator::new(&tx, shard_group, num_preshards(), PROTOCOL_VERSION).unwrap();

    let absent_elsewhere = VersionedSubstateId::new(elsewhere.substate_id().clone(), elsewhere.version().next());
    let proof = generator.generate(&absent_elsewhere).unwrap().expect("shard has state");
    proof
        .verify_exclusion(
            JmtHashScheme::V1,
            PROTOCOL_VERSION,
            &group_root,
            num_preshards(),
            &absent_elsewhere,
        )
        .unwrap();
    proof
        .verify_exclusion(
            JmtHashScheme::V1,
            PROTOCOL_VERSION,
            &group_root,
            num_preshards(),
            &present.to_versioned_substate_id(),
        )
        .unwrap_err();
}

/// A substate the shard group does not cover cannot be proved against its root - which is a fact
/// about the group, not a read failure, so the rest of a batch is still answerable.
#[test]
fn a_substate_outside_the_shard_group_proves_nothing() {
    let (db, _tmp) = create_rocksdb();
    let substates = substates_spanning_shards(8, 2);
    commit_substates(&db, &substates);

    // A shard group holding the first substate's shard but not every substate's.
    let first_shard = substates[0].created().in_shard;
    let shard_group = ShardGroup::new_checked(first_shard, first_shard).unwrap();
    let outsider = substates
        .iter()
        .find(|s| s.created().in_shard != first_shard)
        .expect("substates span more than one shard");

    let tx = db.create_read_tx().unwrap();
    let mut generator = SubstateProofGenerator::new(&tx, shard_group, num_preshards(), PROTOCOL_VERSION).unwrap();

    assert!(
        generator
            .generate(&substates[0].to_versioned_substate_id())
            .unwrap()
            .is_some()
    );
    assert!(
        generator
            .generate(&outsider.to_versioned_substate_id())
            .unwrap()
            .is_none()
    );
}

/// A shard with no committed state has no root to prove against. That is not a read failure, so the
/// responder can drop the one substate and still answer for the rest of its batch.
#[test]
fn a_shard_with_no_committed_state_proves_nothing() {
    let (db, _tmp) = create_rocksdb();
    let shard_group = ShardGroup::all_shards(num_preshards());
    let substates = substates_spanning_shards(2, 2);
    // Commit only the first substate, so the second one's shard has no state tree at all.
    commit_substates(&db, &substates[..1]);

    let tx = db.create_read_tx().unwrap();
    let mut generator = SubstateProofGenerator::new(&tx, shard_group, num_preshards(), PROTOCOL_VERSION).unwrap();

    assert!(
        generator
            .generate(&substates[0].to_versioned_substate_id())
            .unwrap()
            .is_some()
    );
    assert!(
        generator
            .generate(&substates[1].to_versioned_substate_id())
            .unwrap()
            .is_none()
    );
}

/// Across a V1 to V2 activation the shard states stay as they are and only the group root over them
/// changes, so the same store answers for anchors on either side. Each proof holds only at the version
/// of the root it was generated for.
#[test]
fn a_proof_verifies_only_at_the_version_of_its_anchor() {
    let (db, _tmp) = create_rocksdb();
    let shard_group = ShardGroup::all_shards(num_preshards());
    let substates = substates_spanning_shards(4, 2);
    commit_substates(&db, &substates);

    let tx = db.create_read_tx().unwrap();
    let v1_root = shard_group_root_at(&tx, shard_group, ProtocolVersion::V1);
    let v2_root = shard_group_root_at(&tx, shard_group, ProtocolVersion::V2);
    assert_ne!(v1_root, v2_root);
    let mut v1_generator = SubstateProofGenerator::new(&tx, shard_group, num_preshards(), ProtocolVersion::V1).unwrap();
    let mut v2_generator = SubstateProofGenerator::new(&tx, shard_group, num_preshards(), ProtocolVersion::V2).unwrap();

    for substate in &substates {
        let versioned_id = substate.to_versioned_substate_id();
        let value_hash = value_hash(substate);
        let v1_proof = v1_generator.generate(&versioned_id).unwrap().expect("shard has state");
        let v2_proof = v2_generator.generate(&versioned_id).unwrap().expect("shard has state");

        v1_proof
            .verify_inclusion(
                JmtHashScheme::V1,
                ProtocolVersion::V1,
                &v1_root,
                num_preshards(),
                &versioned_id,
                &value_hash,
            )
            .unwrap();
        v2_proof
            .verify_inclusion(
                JmtHashScheme::V1,
                ProtocolVersion::V2,
                &v2_root,
                num_preshards(),
                &versioned_id,
                &value_hash,
            )
            .unwrap();
        v1_proof
            .verify_inclusion(
                JmtHashScheme::V1,
                ProtocolVersion::V2,
                &v2_root,
                num_preshards(),
                &versioned_id,
                &value_hash,
            )
            .unwrap_err();
        v2_proof
            .verify_inclusion(
                JmtHashScheme::V1,
                ProtocolVersion::V1,
                &v1_root,
                num_preshards(),
                &versioned_id,
                &value_hash,
            )
            .unwrap_err();
    }
}

/// At the activation, the first V2 epoch's genesis block commits the last V1 end-of-epoch
/// checkpoint re-rooted under V2. That root must be the one V2 proofs over the same state verify
/// against.
#[test]
fn a_v1_checkpoint_re_rooted_at_v2_anchors_v2_proofs() {
    let (db, _tmp) = create_rocksdb();
    let shard_group = ShardGroup::all_shards(num_preshards());
    let substates = substates_spanning_shards(4, 2);
    commit_substates(&db, &substates);

    let tx = db.create_read_tx().unwrap();
    let checkpoint = checkpoint_of(&tx, shard_group, ProtocolVersion::V1);
    assert_eq!(
        checkpoint.compute_state_merkle_root().unwrap(),
        shard_group_root_at(&tx, shard_group, ProtocolVersion::V1)
    );
    let genesis_root = checkpoint.compute_state_merkle_root_as(ProtocolVersion::V2).unwrap();
    assert_eq!(genesis_root, shard_group_root_at(&tx, shard_group, ProtocolVersion::V2));

    let mut generator = SubstateProofGenerator::new(&tx, shard_group, num_preshards(), ProtocolVersion::V2).unwrap();
    for substate in &substates {
        let versioned_id = substate.to_versioned_substate_id();
        generator
            .generate(&versioned_id)
            .unwrap()
            .expect("shard has state")
            .verify_inclusion(
                JmtHashScheme::V1,
                ProtocolVersion::V2,
                &genesis_root,
                num_preshards(),
                &versioned_id,
                &value_hash(substate),
            )
            .unwrap();
    }
}
