//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Tests for what a node does with the equivocation evidence it holds.

use std::time::Duration;

use ootle_byte_type::ToByteType;
use tari_consensus_types::{BlockId, Decision, ProposalVote, ValidatorSignatureBytes};
use tari_ootle_common_types::{Epoch, NodeHeight, optional::Optional};
use tari_ootle_storage::{
    StateStore,
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
    consensus_models::{Block, ValidatorConsensusStats, VoteEquivocation},
};
use tari_sidechain::QuorumDecision;
use tari_template_lib_types::crypto::{RistrettoPublicKeyBytes, Scalar32Bytes, SchnorrSignatureBytes};

use crate::support::{Test, TestAddress, TestVnDestination, helpers, logging::setup_logger};

fn public_key_of(addr: &TestAddress) -> RistrettoPublicKeyBytes {
    let (_, public_key) = helpers::derive_keypair_from_address(addr);
    public_key.to_byte_type()
}

/// Evidence of the shape the vote collector builds, for a view that has already passed. What it says
/// is not re-verified where it is read: holding it is what counts.
fn evidence_against(accused: &TestAddress, height: NodeHeight) -> VoteEquivocation {
    let public_key = public_key_of(accused);
    let vote = |block_byte: u8| ProposalVote {
        epoch: Epoch(1),
        block_id: BlockId::new(tari_sidechain::FixedHash::new([block_byte; 32])),
        block_height: height,
        decision: QuorumDecision::Accept,
        signature: ValidatorSignatureBytes::new(
            public_key,
            SchnorrSignatureBytes::new([block_byte; 32].into(), Scalar32Bytes::zero()),
        ),
    };

    VoteEquivocation::from_conflicting_votes(Epoch(1), height, public_key, vote(0xA), vote(0xB))
        .expect("the votes name different blocks")
}

/// How many of `validator`'s votes have reached the certificate of a committed block, as the accused
/// - any other validator would do - has committed it.
fn participation_of(test: &Test, validator: &TestAddress) -> u64 {
    let public_key = public_key_of(validator);
    test.validators()[&TestAddress::new("3")]
        .state_store()
        .with_read_tx(|tx| ValidatorConsensusStats::get_by_public_key(tx, Epoch(1), &public_key))
        .optional()
        .unwrap()
        .unwrap_or_default()
        .participation_shares
}

/// A node that holds evidence against the proposer of a block does not sign that block, but it still
/// processes it and commits it once the rest of the committee certifies it. The evidence is local -
/// only the leader of a view sees the conflicting votes for it - so it may not decide anything the
/// committee has to agree on.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_node_holding_evidence_withholds_its_vote_and_consensus_continues() {
    setup_logger();
    let mut test = Test::builder()
        .with_test_timeout(Duration::from_secs(120))
        .modify_consensus_constants(|constants| {
            constants.pacemaker_block_time = Duration::from_secs(2);
        })
        .add_committee(0, vec!["1", "2", "3", "4", "5"])
        .start()
        .await;

    let accused = TestAddress::new("1");
    let accuser = TestAddress::new("2");
    test.start_epoch(Epoch(1)).await;

    let recorded = test.validators()[&accuser]
        .state_store()
        .with_write_tx(|tx| tx.vote_equivocation_record(&evidence_against(&accused, NodeHeight(1))))
        .unwrap();
    assert!(recorded, "evidence was not recorded");
    let holds = test.validators()[&accuser]
        .state_store()
        .with_read_tx(|tx| tx.vote_equivocation_exists_for_validator(Epoch(1), &public_key_of(&accused)))
        .unwrap();
    assert!(holds, "evidence is not readable back");

    // Long enough for the accused to be the leader of a view, and for the block it proposes to be
    // certified by the block above it.
    // A certificate names its signers by their position in the committee, which is ascending by public key.
    let mut committee = ["1", "2", "3", "4", "5"].map(|addr| public_key_of(&TestAddress::new(addr)));
    committee.sort();
    let accuser_index = committee
        .iter()
        .position(|public_key| *public_key == public_key_of(&accuser))
        .unwrap();
    let mut checked_a_block_by_the_accused = false;
    for _ in 0..12 {
        let (tx, _, _) = test.send_transaction_to_all(Decision::Commit, 1, 2, 1).await;
        test.wait_for_transaction_seen(TestVnDestination::All, tx.id()).await;
        let (_, block_id, _, _) = test.on_block_committed().await;

        let store = test.validators()[&accused].state_store();
        let certificate = store
            .with_read_tx(|tx| {
                let block = Block::get(tx, &block_id)?;
                let justified = Block::get(tx, &block.justify().calculate_block_id())?;
                Ok::<_, StorageError>((*justified.proposed_by(), block.justify().clone()))
            })
            .unwrap();
        let (justified_proposer, certificate) = certificate;
        if justified_proposer != public_key_of(&accused) {
            continue;
        }

        // The committee certified the block without the accuser: a quorum is 2f+1 of 5, so the four
        // validators that hold no evidence are enough on their own.
        assert!(
            !certificate.signatures().signers().contains(accuser_index),
            "{accuser} signed a block proposed by {accused} it holds evidence against"
        );
        checked_a_block_by_the_accused = true;
        break;
    }

    assert!(
        checked_a_block_by_the_accused,
        "{accused} never had a block certified, so nothing was proven"
    );

    // Withholding is confined to that proposer: the accuser goes on voting for everyone else, and the
    // committee goes on committing.
    let participation_before = participation_of(&test, &accuser);
    for _ in 0..8 {
        let (tx, _, _) = test.send_transaction_to_all(Decision::Commit, 1, 2, 1).await;
        test.wait_for_transaction_seen(TestVnDestination::All, tx.id()).await;
        test.on_block_committed().await;
    }
    assert!(
        participation_of(&test, &accuser) > participation_before,
        "{accuser} stopped taking part in consensus after withholding one vote"
    );

    test.stop();
    test.assert_clean_shutdown().await;
}
