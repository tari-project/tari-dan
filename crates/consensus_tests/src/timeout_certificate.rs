//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Tests for the rules that bind a proposal to the timeout certificate it carries.
//!
//! A timeout certificate proves that one view failed: the view directly below the proposal, in the proposal's epoch.
//! `check_timeout_certificate_precedes_block` (`crates/consensus/src/validations/common.rs`) holds a proposal to
//! that, so a certificate from another epoch or another view cannot entitle it to skip views.
//!
//! Every timeout vote signs the height of the certificate its signer held when the view failed, so a timeout
//! certificate proves that a quorum has reached that height. The rule is implemented by
//! `check_justify_reaches_timeout_certificate` (`crates/consensus/src/validations/common.rs`): a proposal built on
//! that timeout certificate must justify from a certificate at least that high, so a leader cannot use a leader
//! failure to rewind the committee onto an older branch.

use tari_common_types::types::FixedHash;
use tari_consensus::{
    check_block_commits_to_timeout_certificate,
    check_justify_reaches_timeout_certificate,
    check_timeout_certificate_precedes_block,
    hotstuff::ProposalValidationError,
    messages::NewViewMessage,
};
use tari_consensus_types::{
    BlockId,
    ProposalCertificate,
    ShardGroupAccumulatedData,
    SignedTimeout,
    TimeoutCertificate,
    TimeoutVote,
    ValidatorSignatureBytes,
};
use tari_crypto::tari_utilities::epoch_time::EpochTime;
use tari_ootle_common_types::{Epoch, ExtraData, NodeHeight, NumPreshards, ProtocolVersion, ShardGroup};
use tari_ootle_storage::consensus_models::{Block, BlockCommands, BlockHeader};
use tari_ootle_transaction::Network;
use tari_sidechain::QuorumDecision;
use tari_template_lib_types::crypto::{RistrettoPublicKeyBytes, SchnorrSignatureBytes};

const NUM_PRESHARDS: NumPreshards = NumPreshards::P256;
const NETWORK: Network = Network::LocalNet;
const TEST_EPOCH: Epoch = Epoch(0);

/// A timeout certificate for `height` whose signers attest to `high_pc_heights`.
fn timeout_certificate(height: NodeHeight, high_pc_heights: &[u64]) -> TimeoutCertificate {
    timeout_certificate_in(TEST_EPOCH, height, high_pc_heights)
}

fn timeout_certificate_in(epoch: Epoch, height: NodeHeight, high_pc_heights: &[u64]) -> TimeoutCertificate {
    TimeoutCertificate::new(
        epoch,
        height,
        high_pc_heights
            .iter()
            .map(|h| SignedTimeout {
                high_pc_height: NodeHeight(*h),
                signature: ValidatorSignatureBytes::new(
                    RistrettoPublicKeyBytes::default(),
                    SchnorrSignatureBytes::zero(),
                ),
            })
            .collect(),
    )
}

fn proposal_certificate(height: NodeHeight) -> ProposalCertificate {
    ProposalCertificate::new(
        FixedHash::zero(),
        BlockId::zero(),
        height,
        TEST_EPOCH,
        ShardGroup::all_shards(NUM_PRESHARDS),
        vec![],
        QuorumDecision::Accept,
    )
}

fn timeout_vote(height: NodeHeight, high_pc_height: NodeHeight) -> TimeoutVote {
    TimeoutVote {
        epoch: TEST_EPOCH,
        height,
        high_pc_height,
        signature: ValidatorSignatureBytes::new(RistrettoPublicKeyBytes::default(), SchnorrSignatureBytes::zero()),
    }
}

fn block_justifying(justify_height: NodeHeight, timeout_certificate: TimeoutCertificate) -> Block {
    let height = timeout_certificate.height() + NodeHeight(1);
    block_at(TEST_EPOCH, height, justify_height, timeout_certificate)
}

fn block_at(
    epoch: Epoch,
    height: NodeHeight,
    justify_height: NodeHeight,
    timeout_certificate: TimeoutCertificate,
) -> Block {
    let justify = proposal_certificate(justify_height);
    let header = BlockHeader::create_unsigned(
        NETWORK,
        ProtocolVersion::at(NETWORK, epoch),
        BlockId::zero(),
        justify.calculate_id(),
        Some(timeout_certificate.calculate_id()),
        height,
        epoch,
        ShardGroup::all_shards(NUM_PRESHARDS),
        RistrettoPublicKeyBytes::default(),
        FixedHash::zero(),
        &BlockCommands::empty(),
        0,
        EpochTime::now().as_u64(),
        FixedHash::zero(),
        ShardGroupAccumulatedData::default(),
        ExtraData::new(),
    )
    .unwrap();
    Block::new(header, justify, BlockCommands::empty(), Some(timeout_certificate))
}

#[test]
fn a_timeout_certificate_for_the_preceding_view_is_accepted() {
    let tc = timeout_certificate_in(Epoch(1), NodeHeight(10), &[8, 8, 7]);
    let block = block_at(Epoch(1), NodeHeight(11), NodeHeight(8), tc);

    check_timeout_certificate_precedes_block(&block).unwrap();
}

/// Each timeout is signed over its own epoch, so a certificate from an earlier epoch still verifies against a
/// committee that kept its signers; it proves nothing about a view of the block's epoch.
#[test]
fn a_timeout_certificate_from_an_earlier_epoch_is_rejected() {
    let tc = timeout_certificate_in(Epoch(0), NodeHeight(10), &[8, 8, 7]);
    let block = block_at(Epoch(1), NodeHeight(11), NodeHeight(8), tc);

    let err = check_timeout_certificate_precedes_block(&block).unwrap_err();
    assert!(
        matches!(err, ProposalValidationError::TimeoutCertificateFromAnotherEpoch { .. }),
        "unexpected error: {err}"
    );
}

/// A certificate for an earlier view of the same epoch proves that view failed, not the views the block skips.
#[test]
fn a_timeout_certificate_for_an_earlier_view_is_rejected() {
    let tc = timeout_certificate_in(Epoch(1), NodeHeight(2), &[1, 1, 1]);
    let block = block_at(Epoch(1), NodeHeight(11), NodeHeight(8), tc);

    let err = check_timeout_certificate_precedes_block(&block).unwrap_err();
    assert!(
        matches!(
            err,
            ProposalValidationError::TimeoutCertificateNotForPrecedingView { .. }
        ),
        "unexpected error: {err}"
    );
}

#[test]
fn a_timeout_certificate_at_or_above_the_block_is_rejected() {
    for tc_height in [NodeHeight(11), NodeHeight(12), NodeHeight(u64::MAX)] {
        let tc = timeout_certificate_in(Epoch(1), tc_height, &[8, 8, 7]);
        let block = block_at(Epoch(1), NodeHeight(11), NodeHeight(8), tc);

        let err = check_timeout_certificate_precedes_block(&block).unwrap_err();
        assert!(
            matches!(
                err,
                ProposalValidationError::TimeoutCertificateNotForPrecedingView { .. }
            ),
            "unexpected error for timeout certificate at {tc_height}: {err}"
        );
    }
}

#[test]
fn justify_at_the_attested_height_is_accepted() {
    let tc = timeout_certificate(NodeHeight(9), &[3, 5, 4]);
    let block = block_justifying(NodeHeight(5), tc);

    check_justify_reaches_timeout_certificate(&block).unwrap();
}

/// One signer attesting to a higher certificate is enough: that certificate exists, so the branch below it is
/// settled and a proposal may not justify from under it.
#[test]
fn justify_below_the_attested_height_is_rejected() {
    let tc = timeout_certificate(NodeHeight(9), &[3, 5, 4]);
    let block = block_justifying(NodeHeight(4), tc);

    let err = check_justify_reaches_timeout_certificate(&block).unwrap_err();
    assert!(
        matches!(err, ProposalValidationError::JustifyBelowTimeoutCertificate { .. }),
        "unexpected error: {err}"
    );
}

/// A block with no timeout certificate has nothing to reach.
#[test]
fn a_block_without_a_timeout_certificate_is_accepted() {
    let tc = timeout_certificate(NodeHeight(9), &[3, 5, 4]);
    let mut block = block_justifying(NodeHeight(1), tc);
    block = Block::new(
        block.header().clone(),
        block.justify().clone(),
        BlockCommands::empty(),
        None,
    );

    check_justify_reaches_timeout_certificate(&block).unwrap();
    check_timeout_certificate_precedes_block(&block).unwrap();
}

/// A committee member can self-sign a `SignedTimeout` for any height and add it to a certificate it observes;
/// the certificate still verifies, since each signature is checked against its own message. What stops the
/// splice is the header: it commits to the certificate's id, so the spliced certificate belongs to a different
/// block id than the one the proposer signed.
#[test]
fn a_timeout_certificate_spliced_after_signing_no_longer_matches_the_header() {
    let tc = timeout_certificate(NodeHeight(9), &[3, 5, 4]);
    let block = block_justifying(NodeHeight(5), tc.clone());
    check_block_commits_to_timeout_certificate(&block).unwrap();
    check_justify_reaches_timeout_certificate(&block).unwrap();

    let spliced = timeout_certificate(NodeHeight(9), &[3, 5, 4, 7]);
    assert_ne!(spliced.calculate_id(), tc.calculate_id());
    let spliced_block = Block::new(
        block.header().clone(),
        block.justify().clone(),
        BlockCommands::empty(),
        Some(spliced),
    );
    assert_eq!(
        spliced_block.id(),
        block.id(),
        "the header, and so the block id, is unchanged by the splice"
    );
    let err = check_block_commits_to_timeout_certificate(&spliced_block).unwrap_err();
    assert!(
        matches!(err, ProposalValidationError::TimeoutCertificateIdMismatch { .. }),
        "unexpected error: {err}"
    );

    let rebuilt = block_justifying(NodeHeight(5), timeout_certificate(NodeHeight(9), &[3, 5, 4, 7]));
    assert_ne!(
        rebuilt.id(),
        block.id(),
        "carrying the spliced certificate honestly changes the block id"
    );
}

/// A header that commits to a certificate the block does not carry, or carries one it does not commit to, is
/// rejected the same way.
#[test]
fn a_header_and_block_must_agree_on_whether_there_is_a_timeout_certificate() {
    let tc = timeout_certificate(NodeHeight(9), &[3]);
    let block = block_justifying(NodeHeight(3), tc);
    let dropped = Block::new(
        block.header().clone(),
        block.justify().clone(),
        BlockCommands::empty(),
        None,
    );
    assert!(matches!(
        check_block_commits_to_timeout_certificate(&dropped).unwrap_err(),
        ProposalValidationError::TimeoutCertificateIdMismatch { .. }
    ));
}

/// The claim a timeout vote signs is only worth the certificate carried beside it: every replica other than the
/// one that receives this message takes the signature on trust, so a claim above what the sender can show would
/// bind the committee to a height no leader can reach.
#[test]
fn a_timeout_claim_above_the_carried_certificate_is_not_matched() {
    let high_pc = proposal_certificate(NodeHeight(5));
    let message = NewViewMessage {
        high_pc: high_pc.clone(),
        last_vote: None,
        timeout: timeout_vote(NodeHeight(9), NodeHeight(6)),
    };
    assert!(!message.timeout_claim_matches_high_pc());

    let message = NewViewMessage {
        high_pc,
        last_vote: None,
        timeout: timeout_vote(NodeHeight(9), NodeHeight(5)),
    };
    assert!(message.timeout_claim_matches_high_pc());
}
