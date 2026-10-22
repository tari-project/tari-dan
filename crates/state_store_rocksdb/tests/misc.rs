//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

pub mod helpers;

use helpers::{assert_eq_debug, create_rocksdb};
use indexmap::IndexMap;
use tari_consensus_types::{
    BlockId,
    HighPc,
    LastExecuted,
    LastSentVote,
    LastVoted,
    LeafBlock,
    LockedBlock,
    PcId,
    ProposalVote,
    ValidatorSignatureBytes,
};
use tari_ootle_common_types::{Epoch, NodeHeight, ShardGroup, optional::Optional};
use tari_ootle_storage::{
    StateStore,
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    consensus_models::{Block, EndOfEpochCommand, EpochCheckpoint, TreeRootSummary},
};
use tari_ootle_transaction::Network;
use tari_sidechain::{CommandCommitProof, FixedHash, QuorumDecision, SidechainBlockCommitProof, SidechainBlockHeader};
use tari_state_store_rocksdb::{DatabaseOptions, RocksDbStateStore};
use tari_state_tree::{TreeHash, compute_proof_for_hashes};
use tari_template_lib_types::crypto::{RistrettoPublicKeyBytes, SchnorrSignatureBytes};

use crate::helpers::num_preshards;

#[test]
#[allow(clippy::too_many_lines)]
fn miscellaneous_rocksdb() {
    let (db, _tmp) = create_rocksdb();
    let mut tx = db.create_write_tx().unwrap();

    // last voted
    let mut last_voted = LastVoted {
        block_id: BlockId::zero(),
        height: NodeHeight(123),
        epoch: Epoch::zero(),
    };
    tx.last_voted_set(&last_voted).unwrap();
    let res = tx.last_voted_get(Epoch::zero()).unwrap();
    assert_eq_debug(&res, &last_voted);

    last_voted.epoch += Epoch(1);

    tx.last_voted_set(&last_voted).unwrap();
    let res = tx.last_voted_get(Epoch(1)).unwrap();
    assert_eq_debug(&res, &last_voted);

    // last sent vote
    let mut last_sent_vote = LastSentVote {
        vote: ProposalVote {
            block_id: BlockId::zero(),
            epoch: Epoch::zero(),
            block_height: NodeHeight(123),
            decision: QuorumDecision::Accept,
            signature: ValidatorSignatureBytes::new(RistrettoPublicKeyBytes::default(), SchnorrSignatureBytes::zero()),
        },
    };
    tx.last_sent_vote_set(&last_sent_vote).unwrap();
    let res = tx.last_sent_vote_get(Epoch::zero()).unwrap();
    assert_eq_debug(&res, &last_sent_vote);

    last_sent_vote.vote.epoch += Epoch(1);

    tx.last_sent_vote_set(&last_sent_vote).unwrap();
    let res = tx.last_sent_vote_get(Epoch(1)).unwrap();
    assert_eq_debug(&res, &last_sent_vote);

    // last executed
    let mut last_exec = LastExecuted {
        block_id: BlockId::zero(),
        height: NodeHeight(123),
        epoch: Epoch::zero(),
    };
    tx.last_executed_set(&last_exec).unwrap();
    let res = tx.last_executed_get(Epoch::zero()).unwrap();
    assert_eq_debug(&res, &last_exec);

    last_exec.epoch = Epoch(1);

    tx.last_executed_set(&last_exec).unwrap();
    let res = tx.last_executed_get(Epoch(1)).unwrap();
    assert_eq_debug(&res, &last_exec);

    // last executed
    let mut last_exec = LastExecuted {
        block_id: BlockId::zero(),
        height: NodeHeight(123),
        epoch: Epoch::zero(),
    };
    tx.last_executed_set(&last_exec).unwrap();
    let res = tx.last_executed_get(Epoch::zero()).unwrap();
    assert_eq_debug(&res, &last_exec);

    last_exec.epoch = Epoch(2);

    tx.last_executed_set(&last_exec).unwrap();
    let res = tx.last_executed_get(Epoch(1)).optional().unwrap();
    assert!(res.is_none());
    let res = tx.last_executed_get(Epoch(2)).unwrap();
    assert_eq_debug(&res, &last_exec);

    // locked block
    let epoch = Epoch::zero();
    let mut locked_block = LockedBlock {
        block_id: BlockId::zero(),
        height: NodeHeight(123),
        epoch,
    };
    tx.locked_block_set(&locked_block).unwrap();
    let res = tx.locked_block_get(epoch).unwrap();
    assert_eq_debug(&res, &locked_block);

    locked_block.height += NodeHeight(1);

    tx.locked_block_set(&locked_block).unwrap();
    let res = tx.locked_block_get(epoch).unwrap();
    assert_eq_debug(&res, &locked_block);

    // leaf block
    let epoch = Epoch::zero();
    let mut leaf_block = LeafBlock {
        block_id: BlockId::zero(),
        height: NodeHeight(123),
        epoch,
        shard_group: ShardGroup::all_shards(num_preshards()),
    };
    tx.leaf_block_set(&leaf_block).unwrap();
    let res = tx.leaf_block_get(epoch).unwrap();
    assert_eq_debug(&res, &leaf_block);

    leaf_block.height += NodeHeight(1);

    tx.leaf_block_set(&leaf_block).unwrap();
    let res = tx.leaf_block_get(epoch).unwrap();
    assert_eq_debug(&res, &leaf_block);

    // high qc
    let epoch = Epoch::zero();
    let mut high_qc = HighPc {
        block_id: BlockId::zero(),
        epoch,
        block_height: NodeHeight(123),
        qc_id: PcId::zero(),
    };
    tx.high_pc_set(&high_qc).unwrap();
    let res = tx.high_pc_get(epoch).unwrap();
    assert_eq_debug(&res, &high_qc);

    high_qc.block_height += NodeHeight(1);

    tx.high_pc_set(&high_qc).unwrap();
    let res = tx.high_pc_get(epoch).unwrap();
    assert_eq_debug(&res, &high_qc);

    // epoch checkpoints
    let shard_group = ShardGroup::all_shards(num_preshards());
    let block = Block::zero_block(Network::LocalNet, num_preshards());
    let mut shard_summary = IndexMap::new();
    shard_summary.insert(shard_group.start(), TreeRootSummary {
        root_hash: TreeHash::zero(),
        state_version: 0,
    });
    let key = TreeHash::new([1; 32]);
    let (_, inclusion_proof) = compute_proof_for_hashes([key].into_iter(), key).unwrap();
    let commit_proof = SidechainBlockCommitProof {
        header: SidechainBlockHeader {
            network: 0,
            protocol_version: 0,
            parent_id: Default::default(),
            justify_id: Default::default(),
            height: 0,
            epoch: 0,
            epoch_hash: Default::default(),
            shard_group: tari_sidechain::ShardGroup {
                start: 0,
                end_inclusive: 1,
            },
            proposed_by: Default::default(),
            state_merkle_root: Default::default(),
            command_merkle_root: Default::default(),
            transaction_merkle_root: Default::default(),
            signature: Default::default(),
            accumulated_data: Default::default(),
            metadata_hash: Default::default(),
        },
        proof_elements: vec![],
    };
    let mut next_commit_proof = commit_proof.clone();
    next_commit_proof.header.epoch = 1;
    let next_epoch_checkpoint = EpochCheckpoint::new(
        CommandCommitProof::new(
            EndOfEpochCommand::new(FixedHash::default()),
            next_commit_proof,
            inclusion_proof.clone(),
        ),
        shard_summary.clone(),
        None,
    );
    let mut max_commit_proof = commit_proof.clone();
    max_commit_proof.header.epoch = u64::MAX;
    let max_epoch_checkpoint = EpochCheckpoint::new(
        CommandCommitProof::new(
            EndOfEpochCommand::new(FixedHash::default()),
            max_commit_proof,
            inclusion_proof.clone(),
        ),
        shard_summary.clone(),
        None,
    );
    let proof = CommandCommitProof::new(
        EndOfEpochCommand::new(FixedHash::default()),
        commit_proof,
        inclusion_proof,
    );
    let epoch_checkpoint = EpochCheckpoint::new(proof, shard_summary, Some(block.header().metadata()));

    tx.epoch_checkpoint_save(&epoch_checkpoint).unwrap();
    tx.epoch_checkpoint_save(&next_epoch_checkpoint).unwrap();
    let res = tx
        .epoch_checkpoint_get_all_from_epoch(block.epoch(), 1)
        .unwrap()
        .pop()
        .unwrap();
    assert_eq_debug(&res, &epoch_checkpoint);
    let for_epoch = tx.epoch_checkpoint_get_all_for_epoch(block.epoch()).unwrap();
    assert_eq!(for_epoch.len(), 1);
    assert_eq_debug(&for_epoch[0], &epoch_checkpoint);

    assert!(tx.epoch_checkpoint_get_all_for_epoch(Epoch::max()).unwrap().is_empty());
    tx.epoch_checkpoint_save(&max_epoch_checkpoint).unwrap();
    let for_max_epoch = tx.epoch_checkpoint_get_all_for_epoch(Epoch::max()).unwrap();
    assert_eq!(for_max_epoch.len(), 1);
    assert_eq_debug(&for_max_epoch[0], &max_epoch_checkpoint);

    // foreign parked blocks
    // let justify_qc = QuorumCertificate::genesis(epoch, shard_group);
    // let foreign_parked_block = ForeignParkedProposal::new(ForeignProposalRecord {
    //     block: block.clone(),
    //     block_pledge: BlockPledge::new(),
    //     justify_qc,
    //     proposed_by_block: None,
    //     status: ForeignProposalStatus::New,
    // });
    // tx.foreign_parked_blocks_insert(&foreign_parked_block).unwrap();
    // let res = tx
    //     .foreign_parked_blocks_exists(foreign_parked_block.block().id())
    //     .unwrap();
    // assert!(res);

    tx.rollback().unwrap();
}

/// A durable write transaction commits like any other, and what it wrote is there after the store is reopened.
#[test]
fn a_durable_write_survives_reopening_the_store() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("rocksdb");
    let last_voted = LastVoted {
        block_id: BlockId::zero(),
        height: NodeHeight(7),
        epoch: Epoch(1),
    };

    {
        let db = RocksDbStateStore::<String>::open(&path, DatabaseOptions::default()).unwrap();
        db.with_durable_write_tx(|tx| tx.last_voted_set(&last_voted)).unwrap();
    }

    let db = RocksDbStateStore::<String>::open(&path, DatabaseOptions::default()).unwrap();
    let res = db.with_read_tx(|tx| tx.last_voted_get(Epoch(1))).unwrap();
    assert_eq_debug(&res, &last_voted);
}
