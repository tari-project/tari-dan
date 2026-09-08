//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::BTreeSet;

use ootle_byte_type::ToByteType;
use tari_common_types::types::FixedHash;
use tari_consensus::hotstuff::{
    calculate_dummy_blocks,
    calculate_dummy_blocks_from_justify,
    calculate_last_dummy_block,
};
use tari_consensus_types::{BlockId, ShardGroupAccumulatedData};
use tari_crypto::tari_utilities::hex::Hex;
use tari_ootle_common_types::{
    DerivableFromPublicKey,
    Epoch,
    ExtraData,
    NodeHeight,
    NumPreshards,
    ProtocolVersion,
    ShardGroup,
    VotePower,
    committee::{Committee, CommitteeMember},
    crypto::create_key_pair_from_seed,
};
use tari_ootle_p2p::PeerAddress;
use tari_ootle_storage::consensus_models::{Block, BlockHeader};
use tari_ootle_transaction::Network;

use crate::support::{RoundRobinLeaderStrategy, load_json_fixture};

#[test]
fn dummy_blocks() {
    let shard_group = ShardGroup::new(1, 127);
    let genesis = Block::genesis(
        Network::LocalNet,
        ProtocolVersion::V0,
        Epoch(1),
        FixedHash::zero(),
        shard_group,
        FixedHash::zero(),
        None,
    );
    let committee = (0u8..2)
        .map(create_key_pair_from_seed)
        .map(|(_, pk)| CommitteeMember {
            address: PeerAddress::derive_from_public_key(&pk),
            public_key: pk.to_byte_type(),
            vote_power: VotePower::of(1),
        })
        .collect();

    let dummy = calculate_dummy_blocks(
        NodeHeight(0),
        NodeHeight(30),
        Network::LocalNet,
        Epoch(1),
        shard_group,
        *genesis.id(),
        genesis.justify(),
        genesis.id(),
        FixedHash::zero(),
        &RoundRobinLeaderStrategy,
        &committee,
        genesis.timestamp(),
        ShardGroupAccumulatedData::default(),
        FixedHash::zero(),
    );
    let last = calculate_last_dummy_block(
        NodeHeight(0),
        NodeHeight(30),
        Network::LocalNet,
        Epoch(1),
        shard_group,
        *genesis.id(),
        genesis.justify(),
        FixedHash::zero(),
        &RoundRobinLeaderStrategy,
        &committee,
        genesis.timestamp(),
        ShardGroupAccumulatedData::default(),
        FixedHash::zero(),
    )
    .expect("last dummy block");
    assert_eq!(dummy[0].parent(), genesis.id());
    for i in 1..dummy.len() {
        assert_eq!(dummy[i].parent(), dummy[i - 1].id());
    }
    assert_eq!(dummy.last().unwrap().id(), last.block_id());
    assert_eq!(dummy.len(), 29);
}

#[test]
fn last_matches_generated_using_real_data() {
    let candidate = load_json_fixture::<Block>("block_with_dummies.json");

    let committee = load_json_fixture::<serde_json::Value>("committee.json");
    let committee: Vec<CommitteeMember<PeerAddress>> = serde_json::from_value(committee["members"].clone()).unwrap();
    let committee = Committee::new(committee);

    let justify = Block::genesis(
        Network::LocalNet,
        ProtocolVersion::V0,
        candidate.epoch(),
        FixedHash::zero(),
        candidate.shard_group(),
        FixedHash::zero(),
        None,
    );

    let dummy = calculate_dummy_blocks_from_justify(&candidate, &justify, &RoundRobinLeaderStrategy, &committee);

    let last = calculate_last_dummy_block(
        justify.height(),
        candidate.height(),
        Network::LocalNet,
        justify.epoch(),
        justify.shard_group(),
        *justify.id(),
        justify.justify(),
        *justify.state_merkle_root(),
        &RoundRobinLeaderStrategy,
        &committee,
        justify.timestamp(),
        ShardGroupAccumulatedData::default(),
        *justify.epoch_hash(),
    )
    .expect("last dummy block");

    assert_eq!(dummy.last().unwrap().id(), last.block_id());
}

/// Regression test: when the QC justifies the zero block (no blocks committed yet in the epoch),
/// the proposer must use the epoch genesis block (which has a state_merkle_root and
/// epoch_hash set from the previous epoch checkpoint) rather than the global zero block (all-zero fields). Using the
/// wrong block causes every dummy block ID to diverge, making all proposals permanently invalid.
#[test]
fn dummy_blocks_from_epoch_genesis_vs_zero_block() {
    let shard_group = ShardGroup::all_shards(NumPreshards::P256);
    let non_zero_state_root =
        FixedHash::from_hex("613a7a1b6b83edb2d49c4d740b8b0e7e4ee226b453b004b04d4812dbc51306d9").unwrap();
    let non_zero_epoch_hash =
        FixedHash::from_hex("7da2c68f183ed4a96109e5ebeb18f7e26082f928b9f9879685f90c0bee041451").unwrap();

    // The epoch genesis has real state carried over from the previous epoch
    let epoch_genesis = Block::genesis(
        Network::LocalNet,
        ProtocolVersion::V0,
        Epoch(100),
        non_zero_epoch_hash,
        shard_group,
        non_zero_state_root,
        None,
    );

    // The zero block has all-zero fields - this is what the buggy proposer was using
    let zero_block = Block::zero_block(Network::LocalNet, NumPreshards::P256);

    let committee: Committee<PeerAddress> = (0u8..4)
        .map(create_key_pair_from_seed)
        .map(|(_, pk)| CommitteeMember {
            address: PeerAddress::derive_from_public_key(&pk),
            public_key: pk.to_byte_type(),
            vote_power: VotePower::of(1),
        })
        .collect();

    let candidate_height = NodeHeight(50);
    let qc = epoch_genesis.justify();

    // Simulate the VALIDATOR path: uses epoch genesis (correct)
    let validator_dummies = calculate_dummy_blocks(
        epoch_genesis.height(),
        candidate_height,
        Network::LocalNet,
        Epoch(100),
        shard_group,
        *epoch_genesis.id(),
        qc,
        &BlockId::zero(), // unused expected_parent - we'll check the last dummy directly
        *epoch_genesis.state_merkle_root(),
        &RoundRobinLeaderStrategy,
        &committee,
        epoch_genesis.timestamp(),
        *epoch_genesis.header().accumulated_data(),
        *epoch_genesis.epoch_hash(),
    );

    // Simulate the BUGGY PROPOSER path: uses zero block instead of epoch genesis
    let buggy_proposer_last = calculate_last_dummy_block(
        zero_block.height(),
        candidate_height,
        Network::LocalNet,
        Epoch(100),
        zero_block.shard_group(),
        *zero_block.id(),
        qc,
        *zero_block.state_merkle_root(),
        &RoundRobinLeaderStrategy,
        &committee,
        zero_block.timestamp(),
        *zero_block.header().accumulated_data(),
        *zero_block.epoch_hash(),
    )
    .unwrap();

    // The buggy path produces different dummy block IDs - this was the cause of permanent
    // proposal rejection after leader failure at the start of a new epoch
    assert_ne!(
        validator_dummies.last().unwrap().id(),
        &buggy_proposer_last.block_id,
        "zero block and epoch genesis should produce different dummy chains"
    );

    // Simulate the FIXED PROPOSER path: uses epoch genesis (matches validator)
    let fixed_proposer_last = calculate_last_dummy_block(
        epoch_genesis.height(),
        candidate_height,
        Network::LocalNet,
        Epoch(100),
        epoch_genesis.shard_group(),
        *epoch_genesis.id(),
        qc,
        *epoch_genesis.state_merkle_root(),
        &RoundRobinLeaderStrategy,
        &committee,
        epoch_genesis.timestamp(),
        *epoch_genesis.header().accumulated_data(),
        *epoch_genesis.epoch_hash(),
    )
    .unwrap();

    // The fixed path matches the validator
    assert_eq!(
        validator_dummies.last().unwrap().id(),
        &fixed_proposer_last.block_id,
        "epoch genesis should produce matching dummy chains between proposer and validator"
    );
}

/// Regression test: when the proposer fills a timeout gap with a dummy chain, the candidate's
/// `accumulated_data` must be initialized from the justify block — which the dummies carry
/// forward — and NOT from `highest_seen_block`. This was the second-stage failure observed
/// after the template-sync recovery: HighestSeenBlock had drifted above the high QC because
/// speculative blocks with leader fees had been locally stored on an abandoned fork, so
/// `highest_seen.accumulated > justify.accumulated`. Initialising the proposer candidate from
/// `highest_seen` produced a block header that disagreed with every validator's
/// recomputation from the dummy parent (`Exhaust burn disagreement. Leader proposed N,
/// we calculated M`), and no proposal could ever be voted in.
#[test]
fn proposer_accumulated_data_must_come_from_justify_on_timeout_recovery() {
    let shard_group = ShardGroup::all_shards(NumPreshards::P256);
    let committee: Committee<PeerAddress> = (0u8..4)
        .map(create_key_pair_from_seed)
        .map(|(_, pk)| CommitteeMember {
            address: PeerAddress::derive_from_public_key(&pk),
            public_key: pk.to_byte_type(),
            vote_power: VotePower::of(1),
        })
        .collect();

    // The high QC's referenced block. Its accumulated_data is the canonical baseline
    // for dummy-chain recovery.
    let justify_accumulated = ShardGroupAccumulatedData { total_exhaust_burn: 18 };
    let justify_height = NodeHeight(534);
    let justify_header = BlockHeader::create_unsigned(
        Network::LocalNet,
        ProtocolVersion::V0,
        BlockId::zero(),
        Block::genesis(
            Network::LocalNet,
            ProtocolVersion::V0,
            Epoch(7991),
            FixedHash::zero(),
            shard_group,
            FixedHash::zero(),
            None,
        )
        .justify()
        .calculate_id(),
        justify_height,
        Epoch(7991),
        shard_group,
        committee.shuffled().next().unwrap().public_key,
        FixedHash::zero(),
        &BTreeSet::new(),
        0,
        0,
        FixedHash::zero(),
        justify_accumulated,
        ExtraData::new(),
    )
    .unwrap();
    let justify_block = Block::new(
        justify_header,
        Block::genesis(
            Network::LocalNet,
            ProtocolVersion::V0,
            Epoch(7991),
            FixedHash::zero(),
            shard_group,
            FixedHash::zero(),
            None,
        )
        .justify()
        .clone(),
        BTreeSet::new(),
        None,
    );

    // A locally-stored speculative block on an abandoned fork above the high QC. Its
    // accumulated_data is *higher* than the justify because leader fees got tallied into
    // its header when it was proposed, but it never finalised. HighestSeenBlock follows
    // the local fork, not the QC chain.
    let speculative_accumulated = ShardGroupAccumulatedData {
        total_exhaust_burn: 710,
    };
    assert_ne!(
        speculative_accumulated.total_exhaust_burn, justify_accumulated.total_exhaust_burn,
        "test premise: speculative leaf must disagree with justify"
    );

    // The validator's dummy chain: built from the justify block, so every dummy carries
    // justify's accumulated_data forward — including the last dummy, which becomes the
    // parent of the next candidate.
    let candidate_height = NodeHeight(5304);
    let dummies = calculate_dummy_blocks(
        justify_block.height(),
        candidate_height,
        Network::LocalNet,
        justify_block.epoch(),
        justify_block.shard_group(),
        *justify_block.id(),
        justify_block.justify(),
        // expected_parent_block_id is only used to short-circuit iteration; we want
        // the full chain so pass a placeholder that won't be matched.
        &BlockId::zero(),
        *justify_block.state_merkle_root(),
        &RoundRobinLeaderStrategy,
        &committee,
        justify_block.timestamp(),
        *justify_block.header().accumulated_data(),
        *justify_block.epoch_hash(),
    );

    let last_dummy = dummies.last().expect("dummy chain non-empty");

    // The proposer initialises the candidate's accumulated_data from this value (after
    // the fix). Validators compute total_exhaust_burn starting from this same value
    // (via `parent.header().total_accumulated_exhaust_burn()`). Both sides agree.
    assert_eq!(
        last_dummy.header().accumulated_data().total_exhaust_burn,
        justify_accumulated.total_exhaust_burn,
        "last dummy must carry justify's accumulated_data, not the speculative leaf's"
    );

    // The buggy proposer would have initialised from `highest_seen.accumulated_data`
    // (the speculative leaf), producing a header that disagrees with what every
    // validator computes from the dummy chain.
    assert_ne!(
        last_dummy.header().accumulated_data().total_exhaust_burn,
        speculative_accumulated.total_exhaust_burn,
        "regression: initialising candidate from highest_seen would not match validators"
    );
}

/// Regression test for the proposer-side of the leader-failure recovery fork (defect B).
///
/// After a timeout, the next leader's proposal must extend a block the whole committee
/// deterministically agrees on: the HighQC's justified block, or a dummy chain extending it.
/// Validators reconstruct exactly that chain from the HighQC carried in the candidate
/// (`calculate_dummy_blocks_from_justify`) and require the candidate's parent to equal the last
/// dummy. When the leader's `HighestSeenBlock` is an *uncertified* real block sitting above the
/// HighQC (`leaf > HighPC` — accepted locally but never QC'd, e.g. a re-proposed
/// already-committed transaction whose votes the rest of the committee withholds), the proposer
/// must NOT extend that real block. Its id differs from the dummy every validator recomputes, so
/// the proposal is rejected as `CandidateBlockDoesNotExtendJustify` and the committee forks on
/// every timeout round. The fix anchors the proposer on the HighQC (via `justify_block.height()`),
/// so its parent matches the validators' reconstruction.
#[test]
fn proposer_must_anchor_recovery_on_justify_not_uncertified_leaf() {
    let shard_group = ShardGroup::all_shards(NumPreshards::P256);
    let epoch = Epoch(8516);
    let committee: Committee<PeerAddress> = (0u8..4)
        .map(create_key_pair_from_seed)
        .map(|(_, pk)| CommitteeMember {
            address: PeerAddress::derive_from_public_key(&pk),
            public_key: pk.to_byte_type(),
            vote_power: VotePower::of(1),
        })
        .collect();

    let genesis = Block::genesis(
        Network::LocalNet,
        ProtocolVersion::V0,
        epoch,
        FixedHash::zero(),
        shard_group,
        FixedHash::zero(),
        None,
    );

    let make_block = |parent, height, timestamp| {
        let header = BlockHeader::create_unsigned(
            Network::LocalNet,
            ProtocolVersion::V0,
            parent,
            genesis.justify().calculate_id(),
            height,
            epoch,
            shard_group,
            committee.shuffled().next().unwrap().public_key,
            FixedHash::zero(),
            &BTreeSet::new(),
            0,
            timestamp,
            FixedHash::zero(),
            ShardGroupAccumulatedData::default(),
            ExtraData::new(),
        )
        .unwrap();
        Block::new(header, genesis.justify().clone(), BTreeSet::new(), None)
    };

    // The HighQC's justified block (HighPC), at height H.
    let high_qc_height = NodeHeight(162);
    let justify_block = make_block(*genesis.id(), high_qc_height, 0);
    let gap_height = high_qc_height + NodeHeight(1);
    // After a timeout at H+1, the leader proposes the recovery block at H+2 (one dummy to fill).
    let candidate_height = high_qc_height + NodeHeight(2);

    // The dummy chain every validator reconstructs from the HighQC. The last dummy (at H+1) is the
    // parent the candidate must extend.
    let validator_dummies = calculate_dummy_blocks(
        justify_block.height(),
        candidate_height,
        Network::LocalNet,
        epoch,
        shard_group,
        *justify_block.id(),
        justify_block.justify(),
        &BlockId::zero(), // no early short-circuit; we want the full chain
        *justify_block.state_merkle_root(),
        &RoundRobinLeaderStrategy,
        &committee,
        justify_block.timestamp(),
        *justify_block.header().accumulated_data(),
        *justify_block.epoch_hash(),
    );
    let expected_parent = validator_dummies.last().expect("a dummy block fills the H+1 gap");
    assert_eq!(expected_parent.height(), gap_height);
    assert!(expected_parent.is_dummy());

    // The fix: the proposer anchors on the HighQC, producing the same last dummy as the validator.
    let fixed_parent = calculate_last_dummy_block(
        justify_block.height(),
        candidate_height,
        Network::LocalNet,
        epoch,
        shard_group,
        *justify_block.id(),
        justify_block.justify(),
        *justify_block.state_merkle_root(),
        &RoundRobinLeaderStrategy,
        &committee,
        justify_block.timestamp(),
        *justify_block.header().accumulated_data(),
        *justify_block.epoch_hash(),
    )
    .expect("a dummy block fills the H+1 gap");
    assert_eq!(
        fixed_parent.block_id(),
        expected_parent.id(),
        "fixed proposer (HighQC-anchored) parent must match the validator reconstruction"
    );

    // The buggy proposer extended its HighestSeenBlock instead: a block stored at the gap height that
    // was accepted locally but never gathered a QC (in production, the signed re-proposal of an
    // already-committed transaction). It is a distinct block from the reconstructed dummy.
    let uncertified_leaf = make_block(*justify_block.id(), gap_height, 1);
    assert_eq!(uncertified_leaf.height(), expected_parent.height());

    // Defect B: that real leaf is NOT the dummy validators expect at the gap height, so extending it
    // is rejected as `CandidateBlockDoesNotExtendJustify`, forking the committee on every timeout.
    assert_ne!(
        uncertified_leaf.id(),
        expected_parent.id(),
        "uncertified leaf diverges from the validators' reconstructed dummy — extending it forks the committee"
    );
}
