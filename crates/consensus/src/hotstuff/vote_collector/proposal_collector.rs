//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use log::*;
use tari_consensus_types::{HighPc, ProposalCertificate, ProposalVote, ValidatorSignatureBytes};
use tari_ootle_common_types::{Epoch, NodeHeight, ProtocolVersion, optional::Optional};
use tari_ootle_storage::{StateStore, consensus_models::Block};
use tari_ootle_transaction::Network;
use tari_sidechain::{ProposalVoteMessage, QuorumDecision};

use super::collector::VoteCollector;
use crate::{
    hotstuff::{epoch_state::EpochState, error::HotStuffError, vote_collector::helpers::check_eligibility},
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
}

impl<TConsensusSpec> ProposalVoteCollector<TConsensusSpec>
where TConsensusSpec: ConsensusSpec
{
    pub fn new(
        network: Network,
        store: TConsensusSpec::StateStore,
        epoch_manager: TConsensusSpec::EpochManager,
        vote_signer_service: TConsensusSpec::SignerService,
    ) -> Self {
        Self {
            network,
            store,
            vote_collector: VoteCollector::new(),
            epoch_manager,
            vote_signer_service,
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
        &self,
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
            Err(err) => {
                warn!(target: LOG_TARGET, "❌ {}", err);
                // TODO: store equivocation evidence and punish
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

fn create_proposal_certificate(
    signatures: Vec<ValidatorSignatureBytes>,
    quorum_decision: QuorumDecision,
    block: Block,
) -> ProposalCertificate {
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
