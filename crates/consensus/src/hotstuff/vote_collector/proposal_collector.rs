//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use log::*;
use tari_consensus_types::{HighPc, ProposalCertificate, ProposalVote, ValidatorSignatureBytes};
use tari_ootle_common_types::{Epoch, NodeHeight, ProtocolVersion, optional::Optional};
use tari_ootle_storage::{
    StateStore,
    consensus_models::{Block, VoteEquivocation},
};
use tari_ootle_transaction::Network;
use tari_sidechain::{ProposalVoteMessage, QuorumDecision};

use super::collector::VoteCollector;
use crate::{
    hotstuff::{
        epoch_state::EpochState,
        error::HotStuffError,
        vote_collector::{
            collector::exceeds_vote_lookahead,
            helpers::{check_eligibility, record_equivocation},
        },
    },
    tracing::TraceTimer,
    traits::{CertificateStore, ConsensusSpec, ValidatorSignatureVerifierService},
    validations::signed_vote::SignedProposalVote,
};

const LOG_TARGET: &str = "tari::consensus::hotstuff::proposal_collector";

#[derive(Clone)]
pub struct ProposalVoteCollector<TConsensusSpec: ConsensusSpec> {
    network: Network,
    vote_collector: VoteCollector<ProposalVote>,
    store: TConsensusSpec::StateStore,
    epoch_manager: TConsensusSpec::EpochManager,
    vote_signer_service: TConsensusSpec::SignerService,
    hooks: TConsensusSpec::Hooks,
}

impl<TConsensusSpec> ProposalVoteCollector<TConsensusSpec>
where TConsensusSpec: ConsensusSpec
{
    pub fn new(
        network: Network,
        store: TConsensusSpec::StateStore,
        epoch_manager: TConsensusSpec::EpochManager,
        vote_signer_service: TConsensusSpec::SignerService,
        hooks: TConsensusSpec::Hooks,
    ) -> Self {
        Self {
            network,
            store,
            vote_collector: VoteCollector::new(),
            epoch_manager,
            vote_signer_service,
            hooks,
        }
    }

    pub fn signing_service(&self) -> &TConsensusSpec::SignerService {
        &self.vote_signer_service
    }

    pub fn network(&self) -> Network {
        self.network
    }

    /// Returns Some if quorum is reached
    pub async fn check_and_collect_vote(
        &mut self,
        from: TConsensusSpec::Addr,
        current_height: NodeHeight,
        epoch_state: &EpochState<TConsensusSpec::Addr>,
        vote: ProposalVote,
    ) -> Result<Option<(ProposalCertificate, HighPc)>, HotStuffError> {
        let _timer = TraceTimer::debug(LOG_TARGET, "check_and_collect_vote");
        debug!(
            target: LOG_TARGET,
            "Validating vote message from {from}: {vote}"
        );

        let local_committee_info = epoch_state.local_committee_info();
        let current_epoch = epoch_state.epoch();

        // Before any signature verification: a vote this far ahead is dropped by the collector, so the
        // verification would be work done for nothing.
        if exceeds_vote_lookahead(current_height, vote.block_height) {
            warn!(
                target: LOG_TARGET,
                "🗑️ Discarding {} from {}: it is too far ahead of our current view {}/{}",
                vote,
                from,
                current_epoch,
                current_height
            );
            return Ok(None);
        }

        let block_id = vote.block_id;
        let sender_vn =
            check_eligibility::<TConsensusSpec, _>(&self.epoch_manager, from, &vote, local_committee_info).await?;
        self.validate_vote(current_epoch, &vote)?;
        debug!(
            target: LOG_TARGET,
            "✅ Vote from {} for block {} is valid",
            sender_vn,
            block_id
        );
        let result = self
            .vote_collector
            .collect_vote(
                &sender_vn,
                current_epoch,
                current_height,
                vote,
                epoch_state.local_committee(),
            )
            .await;

        match result {
            Ok(Some((quorum_votes, quorum_decision))) => {
                let Some(block) = self.store.with_read_tx(|tx| Block::get(tx, &block_id).optional())? else {
                    warn!(
                        target: LOG_TARGET,
                        "❓️ Received QUORUM on unknown block {}. Possible race condition where a quorum of votes arrived before the block.",
                        block_id
                    );
                    // The vote will be re-processed when the block arrives and WE vote on it, so there is no special
                    // handling needed
                    self.vote_collector.return_votes(quorum_votes).await;
                    return Ok(None);
                };
                let signatures = quorum_votes.into_iter().map(|vote| vote.signature).collect();
                let new_qc = create_proposal_certificate(signatures, quorum_decision, block);
                let high_qc = self.store.with_write_tx(|tx| new_qc.update_highest(tx))?;
                if new_qc.calculate_id() == *high_qc.id() {
                    info!(target: LOG_TARGET, "🔥 New HIGH {}", new_qc);
                } else {
                    warn!(target: LOG_TARGET, "❓️ New QC from votes {} but it is not the high qc {}", new_qc, high_qc);
                }

                Ok(Some((new_qc, high_qc)))
            },
            Ok(None) => {
                debug!(
                    target: LOG_TARGET,
                    "🟡 No quorum reached yet for ProposalVote at height {}",
                    current_height
                );
                Ok(None)
            },
            Err(duplicate) => {
                warn!(target: LOG_TARGET, "❌ {}", duplicate);
                // A pair that attests to the same block and decision is one vote signed twice, which
                // any signer can do at will and which proves nothing about them.
                let Some(evidence) = VoteEquivocation::from_conflicting_votes(
                    duplicate.epoch,
                    duplicate.height,
                    duplicate.public_key,
                    duplicate.previous_vote,
                    duplicate.new_vote,
                ) else {
                    return Ok(None);
                };
                record_equivocation::<TConsensusSpec>(&self.store, &mut self.hooks, evidence)?;
                Ok(None)
            },
        }
    }

    fn validate_vote(&self, current_epoch: Epoch, vote: &ProposalVote) -> Result<(), HotStuffError> {
        if current_epoch != vote.epoch {
            return Err(HotStuffError::InvalidVote {
                signer_public_key: vote.signature.public_key,
                details: format!(
                    "Our current view is at epoch {} but the vote was for epoch {}",
                    current_epoch, vote.epoch
                ),
            });
        }

        let message = ProposalVoteMessage::new(
            ProtocolVersion::at(self.network, vote.epoch).as_u32(),
            vote.block_id.hash(),
            vote.decision,
            vote.epoch.as_u64(),
            vote.block_height.as_u64(),
        );
        let signed_vote = SignedProposalVote {
            message,
            signature: &vote.signature,
        };
        if !self.vote_signer_service.verify(&signed_vote) {
            return Err(HotStuffError::InvalidVoteSignature {
                signer_public_key: *vote.signature.public_key(),
            });
        }

        Ok(())
    }
}

/// Signatures are ordered by public key so that a node builds the same certificate from the same votes. The order is
/// applied here rather than in `ProposalCertificate::new`, which also decodes received certificates: those must keep
/// the order their proposer committed to, or their recomputed ids would differ from the signed ones.
fn create_proposal_certificate(
    mut signatures: Vec<ValidatorSignatureBytes>,
    quorum_decision: QuorumDecision,
    block: Block,
) -> ProposalCertificate {
    signatures.sort_by(|a, b| a.public_key().cmp(b.public_key()));
    ProposalCertificate::new(
        block.header().calculate_hash(),
        *block.parent(),
        block.height(),
        block.epoch(),
        block.shard_group(),
        signatures,
        quorum_decision,
    )
}

#[cfg(test)]
mod tests {
    use tari_ootle_common_types::NumPreshards;
    use tari_template_lib_types::crypto::{RistrettoPublicKeyBytes, SchnorrSignatureBytes};

    use super::*;

    fn signature(byte: u8) -> ValidatorSignatureBytes {
        let public_key = RistrettoPublicKeyBytes::from_bytes(&[byte; 32]).unwrap();
        ValidatorSignatureBytes::new(public_key, SchnorrSignatureBytes::zero())
    }

    #[test]
    fn it_orders_signatures_by_public_key() {
        let block = Block::zero_block(Network::LocalNet, NumPreshards::P1);
        let orders = [[3, 1, 2], [1, 2, 3], [2, 3, 1]];
        let certificates = orders
            .iter()
            .map(|order| {
                let signatures = order.iter().map(|b| signature(*b)).collect();
                create_proposal_certificate(signatures, QuorumDecision::Accept, block.clone())
            })
            .collect::<Vec<_>>();

        let expected = [signature(1), signature(2), signature(3)];
        for certificate in &certificates {
            assert_eq!(certificate.signatures(), expected);
            assert_eq!(certificate.calculate_id(), certificates[0].calculate_id());
        }
    }
}
