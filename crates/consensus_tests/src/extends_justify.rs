//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Tests for the rule that a candidate block must extend the block its justify certifies.
//!
//! The rule is implemented by `check_extends_justify` (`crates/consensus/src/hotstuff/common.rs`) and is what
//! makes the safeNode predicate a statement about the candidate's whole branch: a candidate certifying one branch
//! while building on another leaves both able to reach a quorum, and both able to commit.
//!
//! A candidate may reach its justify block in exactly two ways: its parent is the justify block, or a timeout
//! certificate proves the views in between failed and the parent is the last of the dummy blocks that every
//! replica recomputes for them.

use tari_common_types::types::FixedHash;
use tari_consensus::hotstuff::{
    LeaderSkipSet,
    ProposalValidationError,
    calculate_dummy_blocks_from_justify,
    check_extends_justify,
};
use tari_consensus_types::{BlockId, LeafBlock, ProposalCertificate, ShardGroupAccumulatedData, TimeoutCertificate};
use tari_crypto::tari_utilities::epoch_time::EpochTime;
use tari_ootle_common_types::{
    Epoch,
    ExtraData,
    NodeHeight,
    NumPreshards,
    ProtocolVersion,
    ShardGroup,
    VotePower,
    committee::{Committee, CommitteeMember},
};
use tari_ootle_storage::consensus_models::{Block, BlockCommands, BlockHeader};
use tari_ootle_transaction::Network;
use tari_sidechain::QuorumDecision;
use tari_template_lib_types::crypto::RistrettoPublicKeyBytes;

use crate::support::{RoundRobinLeaderStrategy, TestAddress};

const NUM_PRESHARDS: NumPreshards = NumPreshards::P256;
const NETWORK: Network = Network::LocalNet;
const TEST_EPOCH: Epoch = Epoch(0);

fn committee() -> Committee<TestAddress> {
    Committee::new(
        ["1", "2", "3"]
            .into_iter()
            .map(|addr| CommitteeMember {
                address: TestAddress::new(addr),
                public_key: RistrettoPublicKeyBytes::default(),
                vote_power: VotePower::of(1),
            })
            .collect(),
    )
}

fn qc_of(target: &LeafBlock) -> ProposalCertificate {
    ProposalCertificate::new(
        *target.block_id().hash(),
        *target.block_id(),
        target.height(),
        target.epoch(),
        ShardGroup::all_shards(NUM_PRESHARDS),
        vec![],
        QuorumDecision::Accept,
    )
}

/// Builds a real block extending `parent_id` at `height` with `justify`. `marker` is mixed into the state Merkle
/// root so that two blocks at the same height produce distinct block ids.
fn build_block(
    parent_id: BlockId,
    justify: ProposalCertificate,
    height: NodeHeight,
    marker: u8,
    timeout_certificate: Option<TimeoutCertificate>,
) -> Block {
    let mut state_root = [0u8; FixedHash::byte_size()];
    state_root[0] = marker;
    state_root[1] = height.as_u64() as u8;
    let header = BlockHeader::create_unsigned(
        NETWORK,
        ProtocolVersion::at(NETWORK, TEST_EPOCH),
        parent_id,
        justify.calculate_id(),
        timeout_certificate.as_ref().map(|tc| tc.calculate_id()),
        height,
        TEST_EPOCH,
        ShardGroup::all_shards(NUM_PRESHARDS),
        RistrettoPublicKeyBytes::default(),
        FixedHash::new(state_root),
        &BlockCommands::empty(),
        0,
        EpochTime::now().as_u64(),
        FixedHash::zero(),
        ShardGroupAccumulatedData::default(),
        ExtraData::new(),
    )
    .unwrap();
    Block::new(header, justify, BlockCommands::empty(), timeout_certificate)
}

fn timeout_certificate_at(height: NodeHeight) -> TimeoutCertificate {
    TimeoutCertificate::new(TEST_EPOCH, height, vec![])
}

/// The dummy chain a proposer must fill `[justify_block.height + 1, candidate_height - 1]` with, as every
/// replica recomputes it.
fn dummy_chain(justify_block: &Block, candidate_height: NodeHeight) -> Vec<Block> {
    let provisional = build_block(
        BlockId::zero(),
        qc_of(&justify_block.as_leaf()),
        candidate_height,
        0,
        Some(timeout_certificate_at(candidate_height - NodeHeight(1))),
    );
    calculate_dummy_blocks_from_justify(
        &provisional,
        justify_block,
        &RoundRobinLeaderStrategy::new(),
        &committee(),
        &LeaderSkipSet::none(),
    )
}

/// A chain with no failed views: the justify block is the parent, and there is nothing to fill in.
fn justify_block() -> Block {
    let zero = Block::zero_block(NETWORK, NUM_PRESHARDS);
    build_block(*zero.id(), zero.justify().clone(), NodeHeight(1), 1, None)
}

#[test]
fn extends_justify_when_parent_is_the_justify_block() {
    let justify_block = justify_block();
    let candidate = build_block(
        *justify_block.id(),
        qc_of(&justify_block.as_leaf()),
        NodeHeight(2),
        1,
        None,
    );

    let dummy_blocks = check_extends_justify(
        &candidate,
        &justify_block,
        &RoundRobinLeaderStrategy::new(),
        &committee(),
        &LeaderSkipSet::none(),
    )
    .unwrap();
    assert!(dummy_blocks.is_empty(), "a contiguous candidate needs no dummy blocks");
}

/// The candidate certifies the justify block's branch but builds on another. Accepting it commits both.
#[test]
fn rejects_parent_that_is_not_the_justify_block() {
    let justify_block = justify_block();
    let zero = Block::zero_block(NETWORK, NUM_PRESHARDS);
    let candidate = build_block(*zero.id(), qc_of(&justify_block.as_leaf()), NodeHeight(2), 2, None);

    let err = check_extends_justify(
        &candidate,
        &justify_block,
        &RoundRobinLeaderStrategy::new(),
        &committee(),
        &LeaderSkipSet::none(),
    )
    .unwrap_err();
    assert!(
        matches!(err, ProposalValidationError::CandidateBlockDoesNotExtendJustify { .. }),
        "unexpected error: {err}"
    );
}

#[test]
fn extends_justify_through_a_full_dummy_chain() {
    let justify_block = justify_block();
    let candidate_height = NodeHeight(5);
    let chain = dummy_chain(&justify_block, candidate_height);
    let candidate = build_block(
        *chain.last().unwrap().id(),
        qc_of(&justify_block.as_leaf()),
        candidate_height,
        1,
        Some(timeout_certificate_at(candidate_height - NodeHeight(1))),
    );

    let dummy_blocks = check_extends_justify(
        &candidate,
        &justify_block,
        &RoundRobinLeaderStrategy::new(),
        &committee(),
        &LeaderSkipSet::none(),
    )
    .unwrap();
    assert_eq!(
        dummy_blocks.len(),
        (candidate_height.as_u64() - justify_block.height().as_u64() - 1) as usize
    );
    assert_eq!(dummy_blocks.last().unwrap().id(), candidate.parent());
}

/// A candidate that parents an earlier dummy skips the views between that dummy and its own height, so it does
/// not extend the chain the proposer claims to have filled.
#[test]
fn rejects_truncated_dummy_chain() {
    let justify_block = justify_block();
    let candidate_height = NodeHeight(5);
    let chain = dummy_chain(&justify_block, candidate_height);
    let earlier_dummy = &chain[chain.len() - 2];
    let candidate = build_block(
        *earlier_dummy.id(),
        qc_of(&justify_block.as_leaf()),
        candidate_height,
        1,
        Some(timeout_certificate_at(candidate_height - NodeHeight(1))),
    );

    let err = check_extends_justify(
        &candidate,
        &justify_block,
        &RoundRobinLeaderStrategy::new(),
        &committee(),
        &LeaderSkipSet::none(),
    )
    .unwrap_err();
    assert!(
        matches!(err, ProposalValidationError::CandidateBlockDoesNotExtendJustify { .. }),
        "unexpected error: {err}"
    );
}

/// Only a timeout certificate proves that the skipped views failed.
#[test]
fn rejects_view_gap_without_a_timeout_certificate() {
    let justify_block = justify_block();
    let candidate_height = NodeHeight(5);
    let chain = dummy_chain(&justify_block, candidate_height);
    let candidate = build_block(
        *chain.last().unwrap().id(),
        qc_of(&justify_block.as_leaf()),
        candidate_height,
        1,
        None,
    );

    let err = check_extends_justify(
        &candidate,
        &justify_block,
        &RoundRobinLeaderStrategy::new(),
        &committee(),
        &LeaderSkipSet::none(),
    )
    .unwrap_err();
    assert!(
        matches!(err, ProposalValidationError::CandidateBlockDoesNotExtendJustify { .. }),
        "unexpected error: {err}"
    );
}

/// The parent is a block the replicas never derive, so nothing links the candidate to the justify block.
#[test]
fn rejects_parent_that_is_not_the_last_dummy_block() {
    let justify_block = justify_block();
    let candidate_height = NodeHeight(5);
    let candidate = build_block(
        *justify_block.id(),
        qc_of(&justify_block.as_leaf()),
        candidate_height,
        1,
        Some(timeout_certificate_at(candidate_height - NodeHeight(1))),
    );

    let err = check_extends_justify(
        &candidate,
        &justify_block,
        &RoundRobinLeaderStrategy::new(),
        &committee(),
        &LeaderSkipSet::none(),
    )
    .unwrap_err();
    assert!(
        matches!(err, ProposalValidationError::CandidateBlockDoesNotExtendJustify { .. }),
        "unexpected error: {err}"
    );
}
