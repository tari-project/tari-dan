//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use log::*;
use tari_common::configuration::Network;
use tari_common_types::types::FixedHash;
use tari_dan_common_types::{committee::CommitteeInfo, optional::Optional, Epoch};
use tari_dan_storage::{
    consensus_models::{Block, HighQc, QuorumCertificate, QuorumDecision, ValidatorSignature, Vote},
    global::models::ValidatorNode,
    StateStore,
};
use tari_epoch_manager::EpochManagerReader;

use crate::{
    hotstuff::error::HotStuffError,
    messages::VoteMessage,
    tracing::TraceTimer,
    traits::{ConsensusSpec, VoteSignatureService},
};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::on_receive_vote";

#[derive(Clone)]
pub struct VoteCollector<TConsensusSpec: ConsensusSpec> {
    network: Network,
    store: TConsensusSpec::StateStore,
    epoch_manager: TConsensusSpec::EpochManager,
    vote_signature_service: TConsensusSpec::SignatureService,
}

impl<TConsensusSpec> VoteCollector<TConsensusSpec>
where TConsensusSpec: ConsensusSpec
{
    pub fn new(
        network: Network,
        store: TConsensusSpec::StateStore,
        epoch_manager: TConsensusSpec::EpochManager,
        vote_signature_service: TConsensusSpec::SignatureService,
    ) -> Self {
        Self {
            network,
            store,
            epoch_manager,
            vote_signature_service,
        }
    }

    /// Returns Some if quorum is reached
    pub async fn check_and_collect_vote(
        &self,
        from: TConsensusSpec::Addr,
        current_epoch: Epoch,
        message: VoteMessage,
        local_committee_info: &CommitteeInfo,
    ) -> Result<Option<(QuorumCertificate, HighQc)>, HotStuffError> {
        let _timer = TraceTimer::debug(LOG_TARGET, "check_and_collect_vote");
        debug!(
            target: LOG_TARGET,
            "📬 Validating vote message from {from}: {message}"
        );

        self.validate_vote_message(current_epoch, &message)?;
        let sender_vn = self.check_eligibility(from, &message, local_committee_info).await?;
        let maybe_qc = self.collect_vote(message, local_committee_info, sender_vn)?;
        if let Some((ref qc, ref high_qc)) = maybe_qc {
            if qc.id() == high_qc.qc_id() {
                info!(target: LOG_TARGET, "🔥 New HIGH {}", qc);
            } else {
                info!(target: LOG_TARGET, "❓️ New QC from votes {} but it is not the high qc {}", qc, high_qc);
            }
        }

        Ok(maybe_qc)
    }

    async fn check_eligibility(
        &self,
        from: <TConsensusSpec as ConsensusSpec>::Addr,
        message: &VoteMessage,
        local_committee_info: &CommitteeInfo,
    ) -> Result<ValidatorNode<<TConsensusSpec as ConsensusSpec>::Addr>, HotStuffError> {
        // Is a local committee member that signed this vote?
        let sender_vn = self
            .epoch_manager
            .get_validator_node_by_public_key(message.epoch, message.signature.public_key.clone())
            .await
            .optional()?;
        let Some(sender_vn) = sender_vn else {
            return Err(HotStuffError::ReceivedVoteFromNonCommitteeMember {
                epoch: message.epoch,
                sender: from.to_string(),
                context: "VoteReceiver::handle_vote (sender pk not from registered VN)".to_string(),
            });
        };

        // Get the sender shard, and check that they are in the local committee
        if !local_committee_info.includes_substate_address(&sender_vn.shard_key) {
            return Err(HotStuffError::ReceivedVoteFromNonCommitteeMember {
                epoch: message.epoch,
                sender: sender_vn.address.to_string(),
                context: "VoteReceiver::handle_vote (VN not in local committee)".to_string(),
            });
        }

        Ok(sender_vn)
    }

    fn collect_vote(
        &self,
        message: VoteMessage,
        local_committee_info: &CommitteeInfo,
        sender_vn: ValidatorNode<TConsensusSpec::Addr>,
    ) -> Result<Option<(QuorumCertificate, HighQc)>, HotStuffError> {
        self.store.with_write_tx(|tx| {
            let sender_leaf_hash = sender_vn.get_node_hash(self.network);

            let exists = Vote {
                epoch: message.epoch,
                block_id: message.block_id,
                decision: message.decision,
                sender_leaf_hash,
                signature: message.signature,
            }
            .save(tx)?;

            if exists {
                warn!(
                    target: LOG_TARGET,
                    "❓️ Received duplicate vote for block {} from {}",
                    message.block_id,
                    sender_vn.address
                );
                return Ok(None);
            }

            let count = Vote::count_for_block(&**tx, &message.block_id)?;
            // We only generate the next high qc once when we have a quorum of votes. Any subsequent votes are not
            // included in the QC.

            info!(
                target: LOG_TARGET,
                "🔥 Received vote for block {} {} {} from {} ({} of {})",
                message.epoch,
                message.unverified_block_height,
                message.block_id,
                sender_vn.address,
                count,
                local_committee_info.quorum_threshold()
            );
            if count != local_committee_info.quorum_threshold() as usize {
                return Ok(None);
            }

            let Some(block) = Block::get(&**tx, &message.block_id).optional()? else {
                warn!(
                    target: LOG_TARGET,
                    "❌ Received {} votes for unknown block {}", count, message.block_id,
                );
                return Ok(None);
            };

            if let Some(existing_qc_for_block) = QuorumCertificate::get_by_block_id(&**tx, block.id()).optional()? {
                debug!(
                    target: LOG_TARGET,
                    "🔥 Received vote for block {} from {} ({} of {}), but we already have a QC for this block ({})",
                    message.block_id,
                    count,
                    sender_vn.address,
                    local_committee_info.quorum_threshold(),
                    existing_qc_for_block
                );
                return Ok(None);
            }

            let votes = block.get_votes(&**tx)?;
            let Some(quorum_decision) = Self::calculate_threshold_decision(&votes, local_committee_info) else {
                warn!(
                    target: LOG_TARGET,
                    "🔥 Received conflicting votes from replicas for block {} ({} of {}). Waiting for more votes.",
                    message.block_id,
                    count,
                    local_committee_info.quorum_threshold()
                );
                return Ok(None);
            };

            let mut signatures = Vec::with_capacity(votes.len());
            let mut leaf_hashes = Vec::with_capacity(votes.len());
            for vote in votes {
                if vote.decision != quorum_decision {
                    // We don't include votes that don't match the quorum decision
                    continue;
                }
                signatures.push(vote.signature);
                leaf_hashes.push(vote.sender_leaf_hash);
            }

            let vote_data = VoteData {
                signatures,
                leaf_hashes,
                quorum_decision,
                block,
            };
            let new_qc = create_qc(vote_data);
            let high_qc = new_qc.update_high_qc(tx)?;

            Ok(Some((new_qc, high_qc)))
        })
    }

    fn calculate_threshold_decision(votes: &[Vote], local_committee_info: &CommitteeInfo) -> Option<QuorumDecision> {
        let mut count_accept = 0;
        let mut count_reject = 0;
        for vote in votes {
            match vote.decision {
                QuorumDecision::Accept => count_accept += 1,
                QuorumDecision::Reject => count_reject += 1,
            }
        }

        let threshold = local_committee_info.quorum_threshold() as usize;
        if count_accept >= threshold {
            return Some(QuorumDecision::Accept);
        }
        if count_reject >= threshold {
            return Some(QuorumDecision::Reject);
        }

        None
    }

    fn validate_vote_message(&self, current_epoch: Epoch, message: &VoteMessage) -> Result<(), HotStuffError> {
        if current_epoch != message.epoch {
            return Err(HotStuffError::InvalidVote {
                signer_public_key: message.signature.public_key.to_string(),
                details: format!(
                    "Our current view is at epoch {} but the vote was for epoch {}",
                    current_epoch, message.epoch
                ),
            });
        }

        if !self
            .vote_signature_service
            .verify(&message.signature, &message.block_id, &message.decision)
        {
            return Err(HotStuffError::InvalidVoteSignature {
                signer_public_key: message.signature.public_key().to_string(),
            });
        }
        Ok(())
    }
}

fn create_qc(vote_data: VoteData) -> QuorumCertificate {
    let VoteData {
        signatures,
        leaf_hashes,
        quorum_decision,
        block,
    } = vote_data;
    QuorumCertificate::new(
        *block.id(),
        block.height(),
        block.epoch(),
        block.shard_group(),
        signatures,
        leaf_hashes,
        quorum_decision,
    )
}

struct VoteData {
    signatures: Vec<ValidatorSignature>,
    leaf_hashes: Vec<FixedHash>,
    quorum_decision: QuorumDecision,
    block: Block,
}
