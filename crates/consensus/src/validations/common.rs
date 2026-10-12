//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause
use std::collections::HashSet;

use log::{debug, warn};
use tari_common_types::types::FixedHash;
use tari_consensus_types::{QuorumCertificateRef, TimeoutVote, ValidatorSignatureBytes};
use tari_engine_types::fees::ExhaustBurnRate;
use tari_ootle_common_types::{
    DerivableFromPublicKey,
    Epoch,
    ExtraFieldKey,
    NodeHeight,
    NumPreshards,
    ProtocolVersion,
    ShardGroup,
    VotePower,
    committee::Committee,
};
use tari_ootle_storage::consensus_models::{Block, BlockHeader};
use tari_ootle_transaction::Network;
use tari_sidechain::ProposalVoteMessage;
use tari_template_lib_types::crypto::RistrettoPublicKeyBytes;

use crate::{
    hotstuff::{HotstuffConfig, LeaderSkipSet, ProposalValidationError},
    traits::{ConsensusSpec, LeaderStrategy, ValidatorSignatureVerifierService},
    validations::signed_vote::SignedProposalVote,
};

const LOG_TARGET: &str = "tari::ootle::consensus::hotstuff::validations";

pub(super) fn check_current_epoch(
    candidate_block: &Block,
    current_epoch: Epoch,
) -> Result<(), ProposalValidationError> {
    if candidate_block.epoch() > current_epoch {
        warn!(target: LOG_TARGET, "⚠️ Proposal for future epoch {} received. Current epoch is {}", candidate_block.epoch(), current_epoch);
        return Err(ProposalValidationError::FutureEpoch {
            block_id: *candidate_block.id(),
            current_epoch,
            block_epoch: candidate_block.epoch(),
        });
    }

    Ok(())
}

pub(super) fn check_network(header: &BlockHeader, network: Network) -> Result<(), ProposalValidationError> {
    if header.network() != network {
        return Err(ProposalValidationError::InvalidNetwork {
            block_network: header.network().to_string(),
            expected_network: network.to_string(),
            block_id: *header.id(),
        });
    }
    Ok(())
}

/// The protocol version a block is produced under is fixed by the network's activation schedule at the block's
/// epoch. A node whose schedule disagrees rejects the block instead of hashing it under a schema the rest of the
/// network has left behind, which stalls the node rather than forking it.
pub(super) fn check_protocol_version(header: &BlockHeader, network: Network) -> Result<(), ProposalValidationError> {
    let expected_version = ProtocolVersion::at(network, header.epoch());
    if header.protocol_version() != expected_version {
        return Err(ProposalValidationError::InvalidProtocolVersion {
            expected_version,
            block_version: header.protocol_version(),
            epoch: header.epoch(),
            block_id: *header.id(),
        });
    }
    Ok(())
}

pub(super) fn check_epoch_hash(
    header: &BlockHeader,
    expected_epoch_hash: &FixedHash,
) -> Result<(), ProposalValidationError> {
    if header.epoch_hash() != expected_epoch_hash {
        return Err(ProposalValidationError::InvalidEpochHash {
            epoch: header.epoch(),
            local_epoch_hash: *expected_epoch_hash,
            invalid_epoch_hash: *header.epoch_hash(),
            block_id: *header.id(),
        });
    }

    Ok(())
}

/// A block must name the rate its epoch runs at, and must name the one this node opened the epoch
/// with.
///
/// The rate is fixed for the whole of an epoch by the end-of-epoch block that opened it, which the
/// committee ratified, so a proposal naming anything else settles fees against a split the rest of
/// the committee will not reproduce.
pub(super) fn check_exhaust_burn_rate(
    header: &BlockHeader,
    expected: ExhaustBurnRate,
) -> Result<(), ProposalValidationError> {
    let Some(rate) = header.try_exhaust_burn_rate() else {
        return Err(ProposalValidationError::MissingExhaustBurnRate {
            block_id: *header.id(),
            epoch: header.epoch(),
            local_rate_bps: expected.as_bps(),
        });
    };

    if rate != expected {
        return Err(ProposalValidationError::InvalidExhaustBurnRate {
            block_id: *header.id(),
            epoch: header.epoch(),
            local_rate_bps: expected.as_bps(),
            invalid_rate_bps: rate.as_bps(),
        });
    }

    Ok(())
}

pub(super) fn check_shard_group_matches(
    header: &BlockHeader,
    expected_shard_group: ShardGroup,
) -> Result<(), ProposalValidationError> {
    if header.shard_group() != expected_shard_group {
        return Err(ProposalValidationError::InvalidShardGroup {
            block_id: *header.id(),
            shard_group: header.shard_group(),
            details: format!(
                "Expected shard group {} but got {}",
                expected_shard_group,
                header.shard_group()
            ),
        });
    }

    Ok(())
}

pub(super) fn check_shard_group_bounds(
    header: &BlockHeader,
    num_preshards: NumPreshards,
) -> Result<(), ProposalValidationError> {
    let len = header
        .shard_group()
        .checked_len()
        .ok_or_else(|| ProposalValidationError::InvalidShardGroup {
            block_id: *header.id(),
            shard_group: header.shard_group(),
            details: "Shard group bounds are invalid".to_string(),
        })?;

    if header.shard_group().start().as_u32() as usize > num_preshards.num_shards() ||
        header.shard_group().end().as_u32() as usize > num_preshards.num_shards()
    {
        return Err(ProposalValidationError::InvalidShardGroup {
            block_id: *header.id(),
            shard_group: header.shard_group(),
            details: format!(
                "Shard group {} is out of bounds for {} preshards",
                header.shard_group(),
                num_preshards.num_shards()
            ),
        });
    }

    if len > num_preshards.num_shards() {
        return Err(ProposalValidationError::InvalidShardGroup {
            block_id: *header.id(),
            shard_group: header.shard_group(),
            details: format!(
                "Shard group {} is larger than the number of preshards {}",
                header.shard_group(),
                num_preshards.num_shards()
            ),
        });
    }

    Ok(())
}

pub(super) fn check_height(block: &Block) -> Result<(), ProposalValidationError> {
    if block.height().is_zero() {
        return Err(ProposalValidationError::InvalidBlockHeight {
            block_id: *block.id(),
            block_height: block.height(),
            details: "Block height is zero".to_string(),
        });
    }
    let max_certificate_height = block.max_certificate_height();
    // invariant: the block may only advance the view by 1 higher than the justified height
    if max_certificate_height.checked_add(NodeHeight(1)) != Some(block.height()) {
        return Err(ProposalValidationError::InvalidBlockHeight {
            block_id: *block.id(),
            block_height: block.height(),
            details: format!("Expected it to be one higher than the max certificate height {max_certificate_height}"),
        });
    }
    Ok(())
}

/// A block may only come from the effective leader of the view below it: the round-robin leader of
/// that view, or - when that validator is suspended - the first validator after it that is not.
///
/// `skip_set` must be the one anchored on the block's justify, which is the state every node that
/// can act on this block has committed.
pub fn check_proposed_by_leader<TAddr: DerivableFromPublicKey, TLeaderStrategy: LeaderStrategy<TAddr>>(
    leader_strategy: &TLeaderStrategy,
    local_committee: &Committee<TAddr>,
    skip_set: &LeaderSkipSet,
    block: &Block,
) -> Result<(), ProposalValidationError> {
    let parent_height =
        block
            .height()
            .checked_sub(NodeHeight(1))
            .ok_or_else(|| ProposalValidationError::InvalidBlockHeight {
                block_id: *block.id(),
                block_height: block.height(),
                details: "Block height is zero".to_string(),
            })?;
    let (addr, leader) = skip_set.effective_leader(leader_strategy, local_committee, parent_height);
    if leader != block.proposed_by() {
        return Err(ProposalValidationError::NotLeader {
            proposed_by: block.proposed_by().to_string(),
            expected_leader: format!("{} / {}", leader, addr),
            block: block.as_leaf(),
            max_certificate_height: block.max_certificate_height(),
        });
    }
    Ok(())
}

pub(super) fn check_block_signature<TSignerService: ValidatorSignatureVerifierService>(
    header: &BlockHeader,
    signer_service: &TSignerService,
) -> Result<(), ProposalValidationError> {
    if header.is_genesis() {
        // Genesis block doesn't have signatures
        return Ok(());
    }

    let validator_signature = header.signature().ok_or(ProposalValidationError::MissingSignature {
        block_id: *header.id(),
        height: header.height(),
    })?;

    debug!(
        target: LOG_TARGET,
        "Validating signature block_id={}, P={}, R={}",
        header.id(),
        header.proposed_by(),
        validator_signature.public_nonce(),
    );

    if !signer_service.verify(header) {
        return Err(ProposalValidationError::InvalidSignature {
            block_id: *header.id(),
            height: header.height(),
        });
    }
    Ok(())
}

pub(super) fn check_proposal_certificate<TConsensusSpec: ConsensusSpec>(
    network: Network,
    candidate_block: &Block,
    committee: &Committee<TConsensusSpec::Addr>,
    signing_service: &TConsensusSpec::SignerService,
) -> Result<(), ProposalValidationError> {
    let qc = candidate_block.justify();
    if candidate_block.height() <= qc.height() {
        return Err(ProposalValidationError::CandidateBlockNotHigherThanJustify {
            justify_block_height: qc.height(),
            candidate_block_height: candidate_block.height(),
        });
    }

    check_quorum_certificate_signatures::<TConsensusSpec>(network, qc.into(), committee, signing_service)?;

    Ok(())
}

pub(super) fn check_timeout_certificate<TConsensusSpec: ConsensusSpec>(
    network: Network,
    candidate_block: &Block,
    committee: &Committee<TConsensusSpec::Addr>,
    signing_service: &TConsensusSpec::SignerService,
) -> Result<(), ProposalValidationError> {
    check_block_commits_to_timeout_certificate(candidate_block)?;
    let Some(tc) = candidate_block.timeout_certificate() else {
        return Ok(());
    };
    check_timeout_certificate_precedes_block(candidate_block)?;

    check_quorum_certificate_signatures::<TConsensusSpec>(network, tc.into(), committee, signing_service)?;

    check_justify_reaches_timeout_certificate(candidate_block)?;

    Ok(())
}

/// Checks that a block's timeout certificate is for the view directly below the block, in the block's epoch.
///
/// A timeout certificate proves that a quorum gave up on one view and nothing more, and it is what entitles a
/// proposer to skip views (`check_extends_justify`). Each timeout is signed over the certificate's own epoch and
/// height, so its signatures verify wherever it is carried; this binding is what ties it to the views the block
/// skips.
pub fn check_timeout_certificate_precedes_block(block: &Block) -> Result<(), ProposalValidationError> {
    let Some(tc) = block.timeout_certificate() else {
        return Ok(());
    };
    if tc.epoch() != block.epoch() {
        return Err(ProposalValidationError::TimeoutCertificateFromAnotherEpoch {
            block_id: *block.id(),
            block_epoch: block.epoch(),
            tc_epoch: tc.epoch(),
        });
    }
    if tc.height().checked_add(NodeHeight(1)) != Some(block.height()) {
        return Err(ProposalValidationError::TimeoutCertificateNotForPrecedingView {
            block_id: *block.id(),
            block_height: block.height(),
            tc_height: tc.height(),
        });
    }
    Ok(())
}

/// Checks that the header's timeout certificate id names the certificate the block carries (or that both are
/// absent).
///
/// What makes a rule that reads the certificate read signed data is the block signature: on decode the header's
/// id is derived from the certificate the block carries (`try_convert_proto_block_header`, as for `justify_id`),
/// so a certificate swapped in flight moves the block id out from under the proposer's signature. A block that
/// arrived over the wire therefore cannot fail here; this guards a locally built block, and it holds the
/// derivation in place should the id ever be sent on the wire instead.
pub fn check_block_commits_to_timeout_certificate(block: &Block) -> Result<(), ProposalValidationError> {
    let header_tc_id = block.header().timeout_certificate_id();
    let tc_id = block.timeout_certificate().map(|tc| tc.calculate_id());
    if header_tc_id != tc_id.as_ref() {
        return Err(ProposalValidationError::TimeoutCertificateIdMismatch {
            block_id: *block.id(),
            header_tc_id: header_tc_id.copied(),
            tc_id,
        });
    }
    Ok(())
}

/// Checks that a block justifies from a certificate at least as high as the highest one its timeout certificate
/// attests to.
///
/// The attested heights carry weight only once the timeout certificate's signatures have been verified against the
/// committee, which `check_timeout_certificate` does before calling this. On a verified certificate a quorum signs
/// the height of the certificate it held when it timed out, so the committee has provably reached that height, and
/// a proposal justifying from lower discards blocks that a quorum has certified - which is how a leader orphans a
/// branch it does not like.
pub fn check_justify_reaches_timeout_certificate(block: &Block) -> Result<(), ProposalValidationError> {
    let Some(tc) = block.timeout_certificate() else {
        return Ok(());
    };
    let max_high_pc_height = tc.max_high_pc_height();
    if block.justify().height() < max_high_pc_height {
        return Err(ProposalValidationError::JustifyBelowTimeoutCertificate {
            block_id: *block.id(),
            justify_height: block.justify().height(),
            max_high_pc_height,
        });
    }
    Ok(())
}

/// Validates the signatures of the quorum certificate.
// pub because used in on receive NEWVIEW
pub fn check_quorum_certificate_signatures<TConsensusSpec: ConsensusSpec>(
    network: Network,
    qc: QuorumCertificateRef<'_>,
    committee: &Committee<TConsensusSpec::Addr>,
    signing_service: &TConsensusSpec::SignerService,
) -> Result<(), ProposalValidationError> {
    // The "zero block" is the deterministic per-epoch genesis, known to every node without a vote, so a
    // QC that justifies it is exempt from quorum/signature validation. Only a `ProposalCertificate` in
    // canonical genesis shape (height zero, zero parent, zero header hash) qualifies for the exemption;
    // everything else - including a `TimeoutCertificate` - falls through to the checks below.
    if let Some(pc) = qc.as_proposal_certificate() &&
        pc.height().is_zero() &&
        pc.signatures().is_empty() &&
        pc.justifies_zero_block() &&
        pc.parent_id().is_zero()
    {
        return Ok(());
    }

    let mut check_dups = HashSet::with_capacity(qc.num_signatures());
    let mut total_vote_power = VotePower::zero();
    let mut account_for = |signature: &ValidatorSignatureBytes| -> Result<(), ProposalValidationError> {
        let Some(power) = committee.get_power_by_public_key(signature.public_key()) else {
            return Err(ProposalValidationError::ValidatorNotInCommittee {
                validator: signature.public_key().to_string(),
                details: format!(
                    "QC {} signed with validator {} that is not in committee",
                    qc,
                    signature.public_key(),
                ),
            });
        };
        total_vote_power += power;
        if !check_dups.insert(*signature.public_key()) {
            return Err(ProposalValidationError::QcDuplicateSignature {
                qc: qc.calculate_id(),
                validator: *signature.public_key(),
            });
        }
        Ok(())
    };

    match qc {
        QuorumCertificateRef::ProposalCertificate(pc) => {
            let block_id = pc.calculate_block_id();
            for signature in pc.signatures() {
                account_for(signature)?;
                // `check_protocol_version` pins a block's version to `at(network, header.epoch())` before any vote
                // is cast on it, so resolving from the schedule here yields the version the signers used.
                let message = ProposalVoteMessage::new(
                    ProtocolVersion::at(network, pc.epoch()).as_u32(),
                    block_id.hash(),
                    pc.decision(),
                    pc.epoch().as_u64(),
                    pc.height().as_u64(),
                );
                let vote = SignedProposalVote { message, signature };
                if !signing_service.verify(&vote) {
                    return Err(ProposalValidationError::QcInvalidSignature { qc: qc.calculate_id() });
                }
            }
        },
        QuorumCertificateRef::TimeoutCertificate(tc) => {
            for timeout in tc.timeouts() {
                account_for(&timeout.signature)?;
                // Each signer attests to its own high certificate height, so the height it reports is part of what
                // it signed and a quorum cannot be assembled that hides one.
                let vote = TimeoutVote {
                    epoch: tc.epoch(),
                    height: tc.height(),
                    high_pc_height: timeout.high_pc_height,
                    signature: timeout.signature.clone(),
                };
                if !signing_service.verify(&vote) {
                    return Err(ProposalValidationError::QcInvalidSignature { qc: qc.calculate_id() });
                }
            }
        },
    }

    if total_vote_power < committee.quorum_threshold() {
        return Err(ProposalValidationError::QuorumWasNotReached {
            qc: qc.calculate_id(),
            got: total_vote_power,
            required: committee.quorum_threshold(),
        });
    }

    Ok(())
}

pub(super) fn check_sidechain_id(header: &BlockHeader, config: &HotstuffConfig) -> Result<(), ProposalValidationError> {
    // We only require the sidechain id on the genesis block
    if !header.is_genesis() {
        return Ok(());
    }

    // If we are using a sidechain id in the network, we need to check it matches the candidate block one
    let Some(expected_sidechain_id) = &config.sidechain_id else {
        return Ok(());
    };

    // Extract the sidechain id from the candidate block
    let extra_data = header.extra_data();
    let sidechain_id_bytes =
        extra_data
            .get(&ExtraFieldKey::SidechainId)
            .ok_or(ProposalValidationError::InvalidSidechainId {
                block_id: *header.id(),
                reason: "SidechainId key not present".to_owned(),
            })?;
    let sidechain_id = RistrettoPublicKeyBytes::from_bytes(sidechain_id_bytes.as_ref()).map_err(|e| {
        ProposalValidationError::InvalidSidechainId {
            block_id: *header.id(),
            reason: e.to_string(),
        }
    })?;

    // The sidechain id must match the sidechain of the current network
    if sidechain_id != *expected_sidechain_id {
        return Err(ProposalValidationError::MismatchedSidechainId {
            block_id: *header.id(),
            expected_sidechain_id: *expected_sidechain_id,
            sidechain_id,
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use tari_consensus_types::{BlockId, ProposalCertificate, ShardGroupAccumulatedData};
    use tari_ootle_common_types::ExtraData;

    use super::*;

    fn header(network: Network, protocol_version: ProtocolVersion, epoch: Epoch) -> BlockHeader {
        let shard_group = ShardGroup::all_shards(NumPreshards::P64);
        BlockHeader::create_unsigned(
            network,
            protocol_version,
            BlockId::zero(),
            ProposalCertificate::genesis(epoch, shard_group).calculate_id(),
            None,
            NodeHeight(2),
            epoch,
            shard_group,
            RistrettoPublicKeyBytes::default(),
            FixedHash::zero(),
            &BTreeSet::new(),
            1,
            1234,
            FixedHash::zero(),
            ShardGroupAccumulatedData::default(),
            ExtraData::new(),
        )
        .unwrap()
    }

    fn is_rejected(network: Network, protocol_version: ProtocolVersion, epoch: Epoch) -> bool {
        matches!(
            check_protocol_version(&header(network, protocol_version, epoch), network),
            Err(ProposalValidationError::InvalidProtocolVersion { .. })
        )
    }

    #[test]
    fn esmeralda_requires_the_version_its_schedule_names() {
        let v1_activation = Epoch(11925);
        let before = Epoch(v1_activation.as_u64() - 1);
        assert!(!is_rejected(Network::Esmeralda, ProtocolVersion::V0, before));
        assert!(is_rejected(Network::Esmeralda, ProtocolVersion::V1, before));
        assert!(!is_rejected(Network::Esmeralda, ProtocolVersion::V1, v1_activation));
        assert!(is_rejected(Network::Esmeralda, ProtocolVersion::V0, v1_activation));
        assert!(is_rejected(Network::Esmeralda, ProtocolVersion::V2, v1_activation));
    }

    #[test]
    fn other_networks_require_v2_from_genesis() {
        for network in [
            Network::MainNet,
            Network::StageNet,
            Network::NextNet,
            Network::Igor,
            Network::LocalNet,
        ] {
            for protocol_version in [ProtocolVersion::V0, ProtocolVersion::V1] {
                assert!(
                    is_rejected(network, protocol_version, Epoch(0)),
                    "{network} {protocol_version}"
                );
            }
            assert!(!is_rejected(network, ProtocolVersion::V2, Epoch(0)), "{network}");
        }
    }
}
