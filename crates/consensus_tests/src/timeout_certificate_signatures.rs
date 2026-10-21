//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Tests for how a timeout certificate's signer bitmap is checked against the committee
//! (`check_quorum_certificate_signatures` in `crates/consensus/src/validations/common.rs`).
//!
//! A certificate names its signers by their position in the committee, which is ascending by public key, and lists
//! one signed timeout per signer in that order. A bitmap that does not fit the committee, or a timeout list that does
//! not match it, is rejected before any signature is verified.

use ootle_byte_type::ToByteType;
use tari_consensus::{
    check_quorum_certificate_signatures,
    hotstuff::ProposalValidationError,
    traits::ValidatorSignerService,
};
use tari_consensus_types::{SignedTimeout, TcSignatures, TimeoutCertificate, TimeoutVoteMessage};
use tari_crypto::tari_utilities::ByteArray;
use tari_ootle_common_types::{
    Epoch,
    NodeHeight,
    VotePower,
    committee::{Committee, CommitteeMember},
};
use tari_ootle_transaction::Network;
use tari_sidechain::SignerBitmap;
use tari_template_lib_types::crypto::RistrettoPublicKeyBytes;

use crate::support::{TestAddress, TestConsensusSpec, TestVoteSignatureService, helpers::derive_keypair_from_address};

const NETWORK: Network = Network::LocalNet;
const EPOCH: Epoch = Epoch(1);
const HEIGHT: NodeHeight = NodeHeight(5);

/// Four members of one vote power each, so three signers reach quorum.
fn committee() -> Committee<TestAddress> {
    ["1", "2", "3", "4"]
        .into_iter()
        .map(|name| {
            let address = TestAddress::new(name);
            let (_, public_key) = derive_keypair_from_address(&address);
            CommitteeMember {
                address,
                public_key: RistrettoPublicKeyBytes::from_bytes(public_key.as_bytes()).unwrap(),
                vote_power: VotePower::of(1),
            }
        })
        .collect()
}

/// The timeout of the committee member at `index`, attesting to a high certificate height of `index`.
fn sign(committee: &Committee<TestAddress>, index: usize) -> SignedTimeout {
    let high_pc_height = NodeHeight(index as u64);
    let message = TimeoutVoteMessage::new(NETWORK, EPOCH, HEIGHT, high_pc_height);
    let signer = TestVoteSignatureService::new(committee.get(index).unwrap().address.clone());
    SignedTimeout {
        high_pc_height,
        signature: signer.sign(&message).to_byte_type(),
    }
}

fn signed_by(committee: &Committee<TestAddress>, indices: &[usize]) -> TcSignatures {
    TcSignatures::Schnorr {
        signers: SignerBitmap::from_indices(committee.len(), indices.iter().copied()).unwrap(),
        timeouts: indices.iter().map(|index| sign(committee, *index)).collect(),
    }
}

fn check(committee: &Committee<TestAddress>, signatures: TcSignatures) -> Result<(), ProposalValidationError> {
    let verifier = TestVoteSignatureService::new(TestAddress::new("verifier"));
    check_quorum_certificate_signatures::<TestConsensusSpec>(
        NETWORK,
        (&TimeoutCertificate::new(EPOCH, HEIGHT, signatures)).into(),
        committee,
        &verifier,
    )
}

#[test]
fn a_quorum_of_signers_verifies() {
    let committee = committee();
    check(&committee, signed_by(&committee, &[0, 2, 3])).unwrap();
}

#[test]
fn signers_short_of_quorum_are_rejected() {
    let committee = committee();
    let err = check(&committee, signed_by(&committee, &[0, 2])).unwrap_err();
    assert!(
        matches!(err, ProposalValidationError::QuorumWasNotReached { .. }),
        "{err}"
    );
}

#[test]
fn a_bitmap_sized_for_another_committee_is_rejected() {
    let committee = committee();
    let TcSignatures::Schnorr { timeouts, .. } = signed_by(&committee, &[0, 1, 2]);
    let signatures = TcSignatures::Schnorr {
        signers: SignerBitmap::from_indices(9, [0, 1, 2]).unwrap(),
        timeouts,
    };
    let err = check(&committee, signatures).unwrap_err();
    assert!(
        matches!(err, ProposalValidationError::QcMalformedSignatures { .. }),
        "{err}"
    );
}

#[test]
fn a_timeout_count_that_differs_from_the_signers_is_rejected() {
    let committee = committee();
    let TcSignatures::Schnorr { signers, mut timeouts } = signed_by(&committee, &[0, 1, 2, 3]);
    timeouts.pop();
    let err = check(&committee, TcSignatures::Schnorr { signers, timeouts }).unwrap_err();
    assert!(
        matches!(err, ProposalValidationError::QcMalformedSignatures { .. }),
        "{err}"
    );
}

#[test]
fn timeouts_out_of_committee_order_are_rejected() {
    let committee = committee();
    let TcSignatures::Schnorr { signers, mut timeouts } = signed_by(&committee, &[0, 1, 2]);
    timeouts.swap(0, 1);
    let err = check(&committee, TcSignatures::Schnorr { signers, timeouts }).unwrap_err();
    assert!(
        matches!(err, ProposalValidationError::QcInvalidSignature { .. }),
        "{err}"
    );
}

#[test]
fn a_bitmap_naming_a_member_who_did_not_sign_is_rejected() {
    let committee = committee();
    let TcSignatures::Schnorr { timeouts, .. } = signed_by(&committee, &[0, 1, 2]);
    let signatures = TcSignatures::Schnorr {
        signers: SignerBitmap::from_indices(committee.len(), [0, 1, 3]).unwrap(),
        timeouts,
    };
    let err = check(&committee, signatures).unwrap_err();
    assert!(
        matches!(err, ProposalValidationError::QcInvalidSignature { .. }),
        "{err}"
    );
}
