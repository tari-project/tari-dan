//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

pub mod helpers;

use std::ops::Deref;

use helpers::{
    NETWORK,
    build_substate_record,
    create_rocksdb_with_opts,
    create_substate_update_batch,
    num_preshards,
    substate_id_seed,
};
use tari_common_types::types::FixedHash;
use tari_consensus_types::BlockId;
use tari_engine_types::ProtocolVersion;
use tari_ootle_common_types::{
    Epoch,
    ShardGroup,
    SubstateVersion,
    ToSubstateAddress,
    VersionedSubstateId,
    optional::Optional,
    shard::Shard,
};
use tari_ootle_storage::{
    DownProofAnchor,
    ShardScopedTreeStoreReader,
    ShardScopedTreeStoreWriter,
    StateStore,
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    SubstateProofGenerator,
    SubstateProofVerifyError,
    consensus_models::{
        CommittedBlockProof,
        StateVersionProof,
        StateVersionProofSource,
        SubstateDestroyed,
        SubstateRecord,
        VerifiedBlockTip,
        index_substate_down_proofs,
        resolve_substate_down_proof,
    },
    verify_substate_down_proof_against_roots,
};
use tari_sidechain::{SidechainBlockCommitProof, SidechainBlockHeader};
use tari_state_store_rocksdb::DatabaseOptions;
use tari_state_tree::{
    SPARSE_MERKLE_PLACEHOLDER_HASH,
    ShardGroupRootTree,
    SpreadPrefixStateTree,
    SubstateTreeChange,
    TreeHash,
    Version,
};

const EPOCH: Epoch = Epoch(1);

/// The tip `commit_proof`'s header describes, taken at its word: these commit proofs are not signed.
fn unvalidated_tip(commit_proof: &CommittedBlockProof) -> VerifiedBlockTip {
    VerifiedBlockTip {
        epoch: commit_proof.epoch(),
        shard_group: commit_proof.shard_group().unwrap(),
        height: commit_proof.height(),
        block_id: commit_proof.block_id(),
        epoch_hash: commit_proof.epoch_hash(),
        state_merkle_root: commit_proof.state_merkle_root(),
    }
}

fn protocol_version() -> ProtocolVersion {
    ProtocolVersion::at(NETWORK, EPOCH)
}

/// The shard-group root tree over the store's committed state.
fn group_tree(tx: &impl StateStoreReadTransaction, shard_group: ShardGroup) -> ShardGroupRootTree {
    let states = shard_group.shard_iter_with_global().map(|shard| {
        let Some(version) = tx.state_tree_versions_get_latest(shard).unwrap() else {
            return (shard, SPARSE_MERKLE_PLACEHOLDER_HASH, 0);
        };
        let mut store = ShardScopedTreeStoreReader::new(tx, shard);
        let root = SpreadPrefixStateTree::new(&mut store).get_root_hash(version).unwrap();
        (shard, root, version)
    });
    ShardGroupRootTree::build(protocol_version(), states.collect::<Vec<_>>()).unwrap()
}

fn commit_proof(shard_group: ShardGroup, height: u64, root: TreeHash) -> CommittedBlockProof {
    let header = SidechainBlockHeader {
        network: NETWORK.as_byte(),
        protocol_version: protocol_version().as_u32(),
        parent_id: FixedHash::zero(),
        justify_id: FixedHash::zero(),
        height,
        epoch: EPOCH.as_u64(),
        epoch_hash: FixedHash::zero(),
        shard_group: tari_sidechain::ShardGroup {
            start: shard_group.start().as_u32(),
            end_inclusive: shard_group.end().as_u32(),
        },
        proposed_by: Default::default(),
        state_merkle_root: FixedHash::new(root.into_array()),
        command_merkle_root: FixedHash::zero(),
        transaction_merkle_root: None,
        signature: Default::default(),
        accumulated_data: Default::default(),
        metadata_hash: FixedHash::zero(),
    };
    CommittedBlockProof::new(SidechainBlockCommitProof {
        header,
        proof_elements: vec![],
    })
}

/// Writes `changes` to `shard`'s tree at `version` and commits the substate records, recording a down proof for each
/// substate the batch destroys, as a validator does when a block commits.
fn commit_version<TTx>(
    tx: &mut TTx,
    shard: Shard,
    version: Version,
    changes: Vec<SubstateTreeChange>,
    records: &[&SubstateRecord],
    commit_proof_for_block: impl FnMut(&TTx::Target, &BlockId) -> Option<Vec<u8>>,
) where
    TTx: StateStoreWriteTransaction + Deref,
    TTx::Target: StateStoreReadTransaction,
{
    {
        let mut store = ShardScopedTreeStoreWriter::new(tx, shard);
        SpreadPrefixStateTree::new(&mut store)
            .batch_put_substate_changes((version > 1).then(|| version - 1), version, changes)
            .unwrap();
    }
    tx.state_tree_shard_versions_set(shard, version).unwrap();
    let batch = create_substate_update_batch(EPOCH, records.iter().copied());
    let downed = batch.downed();
    tx.substates_commit_batch(batch).unwrap();
    index_substate_down_proofs(tx, &downed, commit_proof_for_block).unwrap();
}

fn up(record: &SubstateRecord) -> SubstateTreeChange {
    SubstateTreeChange::Up {
        id: record.to_versioned_substate_id(),
        value_hash: *record.state_hash(),
    }
}

fn destroyed(record: &SubstateRecord, at_state_version: Version) -> SubstateRecord {
    let mut record = record.clone();
    record.destroyed = Some(SubstateDestroyed {
        at_epoch: EPOCH,
        at_state_version,
    });
    record
}

/// Serves the recorded down proof of `target_id` against the store's latest state, as the RPC does, and verifies it
/// with `r1` and a commit proof of the latest state at `height`.
fn verify_down_at_latest<S: StateStore>(
    db: &S,
    shard_group: ShardGroup,
    target_id: &VersionedSubstateId,
    r1: &DownProofAnchor,
    height: u64,
) -> Result<(), SubstateProofVerifyError> {
    let tx = db.create_read_tx().unwrap();
    let record = tx
        .substate_down_proofs_get(target_id.to_shard(num_preshards()), target_id)
        .unwrap()
        .unwrap();
    assert_eq!(record.state_version, 1);
    let r2_commit_proof = commit_proof(shard_group, height, group_tree(&tx, shard_group).root());
    let down = SubstateProofGenerator::new(&tx, shard_group, num_preshards(), protocol_version())
        .unwrap()
        .generate(target_id)
        .unwrap()
        .unwrap();
    let proof = resolve_substate_down_proof(&tx, target_id.to_shard(num_preshards()), record, down)
        .unwrap()
        .expect("the record's state version proof and its commit proof are held");
    let proof = tari_bor::serde_codec::to_vec(proof).unwrap();
    verify_substate_down_proof_against_roots(
        &proof,
        target_id.substate_id(),
        target_id.version(),
        NETWORK,
        num_preshards(),
        r1,
        &DownProofAnchor::from(unvalidated_tip(&r2_commit_proof)),
    )
}

fn no_committed_blocks<T: ?Sized>(_: &T, _: &BlockId) -> Option<Vec<u8>> {
    None
}

/// Commits `records` as the shard's first version and records a received proof of that version at height 2.
fn commit_with_received_proof<S: StateStore>(
    db: &S,
    shard_group: ShardGroup,
    shard: Shard,
    records: &[&SubstateRecord],
) -> CommittedBlockProof {
    let mut tx = db.create_write_tx().unwrap();
    let changes = records.iter().map(|record| up(record)).collect();
    commit_version(&mut tx, shard, 1, changes, records, no_committed_blocks);
    let tree = group_tree(&*tx, shard_group);
    let commit_proof = commit_proof(shard_group, 2, tree.root());
    tx.state_version_proofs_insert(&StateVersionProof {
        shard,
        state_version: 1,
        source: StateVersionProofSource::Received {
            commit_proof: commit_proof.to_bytes(),
        },
        shard_root_proof: tree.get_proof(shard).unwrap().1,
    })
    .unwrap();
    tx.commit().unwrap();
    commit_proof
}

#[test]
fn a_destroyed_substate_stays_provably_down_after_its_state_is_pruned() {
    let (db, _dir) = create_rocksdb_with_opts(DatabaseOptions::default().with_state_history_length(1));

    const SEED: u32 = 1;
    let target = build_substate_record(&substate_id_seed(SEED << 24), SubstateVersion::ZERO, 1);
    let shard = target.created().in_shard;
    let shard_group = ShardGroup::new(shard, shard);
    let target_id = target.to_versioned_substate_id();
    let neighbour = build_substate_record(&substate_id_seed((SEED << 24) | 1), SubstateVersion::ZERO, 1);

    // State version 1: the target is created; this node holds a proof of the shard at it, as a synced node does at
    // a proof point.
    let r1_commit_proof = commit_with_received_proof(&db, shard_group, shard, &[&target, &neighbour]);
    let r1 = DownProofAnchor::from(unvalidated_tip(&r1_commit_proof));

    // State version 2: the target is destroyed and its next version created.
    let next = build_substate_record(target.substate_id(), SubstateVersion::new(1), 2);
    {
        let mut tx = db.create_write_tx().unwrap();
        commit_version(
            &mut tx,
            shard,
            2,
            vec![SubstateTreeChange::Down { id: target_id.clone() }, up(&next)],
            &[&destroyed(&target, 2), &next],
            no_committed_blocks,
        );
        tx.commit().unwrap();
    }

    verify_down_at_latest(&db, shard_group, &target_id, &r1, 4).unwrap();

    // State version 3: the next version is destroyed. It was created at state version 2, a version this node holds
    // no usable proof of: its only candidate is a block this node can no longer build a commit proof for.
    {
        let mut tx = db.create_write_tx().unwrap();
        tx.state_version_proofs_insert(&StateVersionProof {
            shard,
            state_version: 2,
            source: StateVersionProofSource::Committed {
                block_id: BlockId::zero(),
            },
            shard_root_proof: group_tree(&*tx, shard_group).get_proof(shard).unwrap().1,
        })
        .unwrap();
        commit_version(
            &mut tx,
            shard,
            3,
            vec![SubstateTreeChange::Down {
                id: next.to_versioned_substate_id(),
            }],
            &[&destroyed(&next, 3)],
            no_committed_blocks,
        );
        tx.commit().unwrap();
    }
    {
        let tx = db.create_read_tx().unwrap();
        assert!(
            tx.substate_down_proofs_get(shard, &next.to_versioned_substate_id())
                .unwrap()
                .is_none()
        );
    }

    // Prune the tree nodes of v1 and the target's value.
    {
        let mut tx = db.create_write_tx().unwrap();
        assert!(tx.state_tree_nodes_clear_stale(num_preshards(), usize::MAX).unwrap() > 0);
        tx.substates_prune_downed_values(EPOCH, usize::MAX).unwrap();
        tx.commit().unwrap();
    }
    {
        let tx = db.create_read_tx().unwrap();
        let mut store = ShardScopedTreeStoreReader::new(&tx, shard);
        assert!(
            SpreadPrefixStateTree::new(&mut store).get_proof(1, &target_id).is_err(),
            "v1 is still in the tree"
        );
        let record = SubstateRecord::get(&tx, &target_id.to_substate_address()).unwrap();
        assert!(record.substate_value().is_none(), "the target's value was not pruned");
    }

    verify_down_at_latest(&db, shard_group, &target_id, &r1, 6).unwrap();
}

#[test]
fn a_substate_destroyed_with_a_committed_block_proof_is_recorded() {
    let (db, _dir) = create_rocksdb_with_opts(DatabaseOptions::default());

    const SEED: u32 = 2;
    let target = build_substate_record(&substate_id_seed(SEED << 24), SubstateVersion::ZERO, 1);
    let shard = target.created().in_shard;
    let shard_group = ShardGroup::new(shard, shard);
    let target_id = target.to_versioned_substate_id();
    let neighbour = build_substate_record(&substate_id_seed((SEED << 24) | 1), SubstateVersion::ZERO, 1);

    let block_id = BlockId::from([7u8; 32]);
    let mut tx = db.create_write_tx().unwrap();
    commit_version(
        &mut tx,
        shard,
        1,
        vec![up(&target), up(&neighbour)],
        &[&target, &neighbour],
        no_committed_blocks,
    );
    let tree = group_tree(&*tx, shard_group);
    let r1_commit_proof = commit_proof(shard_group, 2, tree.root());
    tx.state_version_proofs_insert(&StateVersionProof {
        shard,
        state_version: 1,
        source: StateVersionProofSource::Committed { block_id },
        shard_root_proof: tree.get_proof(shard).unwrap().1,
    })
    .unwrap();

    let commit_proof_bytes = r1_commit_proof.to_bytes();
    commit_version(
        &mut tx,
        shard,
        2,
        vec![SubstateTreeChange::Down { id: target_id.clone() }],
        &[&destroyed(&target, 2)],
        |_, id| (*id == block_id).then(|| commit_proof_bytes.clone()),
    );

    let record = tx.substate_down_proofs_get(shard, &target_id).unwrap().unwrap();
    assert_eq!(record.state_version, 1);
    assert_eq!(record.value_hash, TreeHash::new(target.state_hash().into_array()));
    // No commit proof was stored for the block, so indexing built one and stored it per block, where the record's
    // state version proof finds it once the block is pruned.
    assert_eq!(
        tx.block_commit_proofs_get(&block_id).unwrap(),
        r1_commit_proof.to_bytes()
    );
    let r1 = DownProofAnchor::from(unvalidated_tip(&r1_commit_proof));
    let up = resolve_substate_down_proof(
        &*tx,
        shard,
        record,
        SubstateProofGenerator::new(&*tx, shard_group, num_preshards(), protocol_version())
            .unwrap()
            .generate(&target_id)
            .unwrap()
            .unwrap(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(up.up_commit_proof, r1_commit_proof.to_bytes());
    up.up
        .verify_inclusion(
            tari_state_tree::jmt_hash_scheme(protocol_version()),
            protocol_version(),
            &TreeHash::new(r1.root().into_array()),
            num_preshards(),
            &target_id,
            &up.up_value_hash,
        )
        .unwrap();
}

/// Substates a block takes down together in one shard are proved up at one version, whose block's commit proof is
/// built at most once and stored once, per block, beside the commit proofs consensus stores.
#[test]
fn substates_destroyed_together_share_one_commit_proof() {
    for already_stored in [false, true] {
        let (db, _dir) = create_rocksdb_with_opts(DatabaseOptions::default());

        const SEED: u32 = 3;
        let targets = (0..3)
            .map(|n| build_substate_record(&substate_id_seed((SEED << 24) | n), SubstateVersion::ZERO, 1))
            .collect::<Vec<_>>();
        let shard = targets[0].created().in_shard;
        let shard_group = ShardGroup::new(shard, shard);
        let neighbour = build_substate_record(&substate_id_seed((SEED << 24) | 100), SubstateVersion::ZERO, 1);
        assert!(
            targets
                .iter()
                .chain([&neighbour])
                .all(|t| t.created().in_shard == shard)
        );

        let block_id = BlockId::from([9u8; 32]);
        let mut tx = db.create_write_tx().unwrap();
        let mut v1 = targets.iter().collect::<Vec<_>>();
        v1.push(&neighbour);
        commit_version(
            &mut tx,
            shard,
            1,
            v1.iter().map(|record| up(record)).collect(),
            &v1,
            no_committed_blocks,
        );
        let tree = group_tree(&*tx, shard_group);
        let commit_proof_bytes = commit_proof(shard_group, 2, tree.root()).to_bytes();
        tx.state_version_proofs_insert(&StateVersionProof {
            shard,
            state_version: 1,
            source: StateVersionProofSource::Committed { block_id },
            shard_root_proof: tree.get_proof(shard).unwrap().1,
        })
        .unwrap();
        if already_stored {
            // As consensus stores it when the block commits.
            tx.block_commit_proofs_insert(&block_id, &commit_proof_bytes).unwrap();
        }

        let destroyed_records = targets.iter().map(|t| destroyed(t, 2)).collect::<Vec<_>>();
        let mut builds = 0;
        commit_version(
            &mut tx,
            shard,
            2,
            targets
                .iter()
                .map(|t| SubstateTreeChange::Down {
                    id: t.to_versioned_substate_id(),
                })
                .collect(),
            &destroyed_records.iter().collect::<Vec<_>>(),
            |_, id| {
                builds += 1;
                (*id == block_id).then(|| commit_proof_bytes.clone())
            },
        );

        assert_eq!(builds, usize::from(!already_stored), "already stored: {already_stored}");
        for target in &targets {
            let record = tx
                .substate_down_proofs_get(shard, &target.to_versioned_substate_id())
                .unwrap()
                .unwrap();
            assert_eq!(record.state_version, 1);
        }
        assert_eq!(tx.block_commit_proofs_get(&block_id).unwrap(), commit_proof_bytes);
    }
}

/// A received commit proof is served from the state version proof that carries it, so nothing is stored for it.
#[test]
fn a_received_commit_proof_is_served_from_its_state_version_proof() {
    let (db, _dir) = create_rocksdb_with_opts(DatabaseOptions::default());

    const SEED: u32 = 4;
    let targets = (0..2)
        .map(|n| build_substate_record(&substate_id_seed((SEED << 24) | n), SubstateVersion::ZERO, 1))
        .collect::<Vec<_>>();
    let shard = targets[0].created().in_shard;
    let shard_group = ShardGroup::new(shard, shard);
    let neighbour = build_substate_record(&substate_id_seed((SEED << 24) | 100), SubstateVersion::ZERO, 1);
    let mut v1 = targets.iter().collect::<Vec<_>>();
    v1.push(&neighbour);
    let r1_commit_proof = commit_with_received_proof(&db, shard_group, shard, &v1);

    let mut tx = db.create_write_tx().unwrap();
    let destroyed_records = targets.iter().map(|t| destroyed(t, 2)).collect::<Vec<_>>();
    commit_version(
        &mut tx,
        shard,
        2,
        targets
            .iter()
            .map(|t| SubstateTreeChange::Down {
                id: t.to_versioned_substate_id(),
            })
            .collect(),
        &destroyed_records.iter().collect::<Vec<_>>(),
        no_committed_blocks,
    );

    let exclusion = SubstateProofGenerator::new(&*tx, shard_group, num_preshards(), protocol_version())
        .unwrap()
        .generate(&targets[0].to_versioned_substate_id())
        .unwrap()
        .unwrap();
    for target in &targets {
        let record = tx
            .substate_down_proofs_get(shard, &target.to_versioned_substate_id())
            .unwrap()
            .unwrap();
        let proof = resolve_substate_down_proof(&*tx, shard, record, exclusion.clone())
            .unwrap()
            .unwrap();
        assert_eq!(proof.up_commit_proof, r1_commit_proof.to_bytes());
    }
    assert!(
        tx.block_commit_proofs_get(&BlockId::new(r1_commit_proof.block_id()))
            .optional()
            .unwrap()
            .is_none()
    );
}
