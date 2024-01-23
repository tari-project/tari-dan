//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::ops::DerefMut;

use log::*;
use tari_common_types::types::FixedHash;
use tari_dan_common_types::{committee::CommitteeShard, hashing::MergedValidatorNodeMerkleProof, optional::Optional};
use tari_dan_storage::{
    consensus_models::{Block, QuorumCertificate, QuorumDecision, ValidatorSignature, Vote},
    StateStore,
};
use tari_epoch_manager::EpochManagerReader;

use crate::{
    hotstuff::{error::HotStuffError, pacemaker_handle::PaceMakerHandle},
    messages::VoteMessage,
    traits::{ConsensusSpec, LeaderStrategy, VoteSignatureService},
};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::on_receive_vote";

#[derive(Clone)]
pub struct VoteReceiver<TConsensusSpec: ConsensusSpec> {
    store: TConsensusSpec::StateStore,
    leader_strategy: TConsensusSpec::LeaderStrategy,
    epoch_manager: TConsensusSpec::EpochManager,
    vote_signature_service: TConsensusSpec::SignatureService,
    pacemaker: PaceMakerHandle,
}

impl<TConsensusSpec> VoteReceiver<TConsensusSpec>
where TConsensusSpec: ConsensusSpec
{
    pub fn new(
        store: TConsensusSpec::StateStore,
        leader_strategy: TConsensusSpec::LeaderStrategy,
        epoch_manager: TConsensusSpec::EpochManager,
        vote_signature_service: TConsensusSpec::SignatureService,
        pacemaker: PaceMakerHandle,
    ) -> Self {
        Self {
            store,
            leader_strategy,
            epoch_manager,
            pacemaker,
            vote_signature_service,
        }
    }

    pub async fn handle(
        &self,
        from: TConsensusSpec::Addr,
        message: VoteMessage,
        check_leadership: bool,
    ) -> Result<(), HotStuffError> {
        match self.handle_vote(from, message, check_leadership).await {
            Ok(true) => {
                // If we reached quorum, trigger a check to see if we should propose
                self.pacemaker.beat();
            },
            Ok(false) => {},
            Err(err) => {
                // We dont want bad vote messages to kick us out of running mode
                warn!(target: LOG_TARGET, "❌ Error handling vote: {}", err);
            },
        }
        Ok(())
    }

    /// Returns true if quorum is reached
    #[allow(clippy::too_many_lines)]
    pub async fn handle_vote(
        &self,
        from: TConsensusSpec::Addr,
        message: VoteMessage,
        check_leadership: bool,
    ) -> Result<bool, HotStuffError> {
        let current_epoch = self.epoch_manager.current_epoch().await?;
        // Is a committee member sending us this vote?
        let committee = self.epoch_manager.get_local_committee(current_epoch).await?;
        if !committee.contains(&from) {
            return Err(HotStuffError::ReceivedMessageFromNonCommitteeMember {
                epoch: current_epoch,
                sender: from.to_string(),
                context: "OnReceiveVote".to_string(),
            });
        }

        // Are we the leader for the block being voted for?
        let our_vn = self.epoch_manager.get_our_validator_node(current_epoch).await?;

        let local_committee_shard = self.epoch_manager.get_local_committee_shard(current_epoch).await?;

        // Get the sender shard, and check that they are in the local committee
        let sender_vn = self.epoch_manager.get_validator_node(current_epoch, &from).await?;
        if message.signature.public_key != sender_vn.public_key {
            return Err(HotStuffError::RejectingVoteNotSentBySigner {
                address: from.to_string(),
                signer_public_key: message.signature.public_key.to_string(),
            });
        }

        if !local_committee_shard.includes_substate_address(&sender_vn.shard_key) {
            return Err(HotStuffError::ReceivedMessageFromNonCommitteeMember {
                epoch: current_epoch,
                sender: message.signature.public_key.to_string(),
                context: "OnReceiveVote".to_string(),
            });
        }

        let sender_leaf_hash = sender_vn.node_hash();

        self.validate_vote_message(&message, &sender_leaf_hash)?;

        let from = message.signature.public_key.clone();

        let count = self.store.with_write_tx(|tx| {
            Vote {
                epoch: message.epoch,
                block_id: message.block_id,
                decision: message.decision,
                sender_leaf_hash,
                signature: message.signature,
            }
            .save(tx)?;

            let count = Vote::count_for_block(tx.deref_mut(), &message.block_id)?;
            Ok::<_, HotStuffError>(count)
        })?;

        // We only generate the next high qc once when we have a quorum of votes. Any subsequent votes are not included
        // in the QC.

        info!(
            target: LOG_TARGET,
            "🔥 Received vote for block #{} {} from {} ({} of {})",
            message.block_height,
            message.block_id,
            from,
            count,
            local_committee_shard.quorum_threshold()
        );
        if count < local_committee_shard.quorum_threshold() as usize {
            return Ok(false);
        }

        let vote_data;
        {
            let mut tx = self.store.create_read_tx()?;
            let Some(block) = Block::get(&mut tx, &message.block_id).optional()? else {
                warn!(
                    target: LOG_TARGET,
                    "❌ Received {} votes for unknown block {}", count, message.block_id
                );
                return Ok(false);
            };

            if check_leadership &&
                !self
                    .leader_strategy
                    .is_leader_for_next_block(&our_vn.address, &committee, block.height())
            {
                return Err(HotStuffError::NotTheLeader {
                    details: format!(
                        "Not this leader for block {}, vote sent by {}",
                        message.block_id, our_vn.address
                    ),
                });
            }

            if let Some(existing_qc_for_block) = QuorumCertificate::get_by_block_id(&mut tx, block.id()).optional()? {
                debug!(
                    target: LOG_TARGET,
                    "🔥 Received vote for block {} from {} ({} of {}), but we already have a QC for this block ({})",
                    message.block_id,
                    from,
                    count,
                    local_committee_shard.quorum_threshold(),
                    existing_qc_for_block
                );
                return Ok(true);
            }

            let votes = block.get_votes(&mut tx)?;
            let Some(quorum_decision) = Self::calculate_threshold_decision(&votes, &local_committee_shard) else {
                warn!(
                    target: LOG_TARGET,
                    "🔥 Received conflicting votes from replicas for block {} ({} of {}). Waiting for more votes.",
                    message.block_id,
                    count,
                    local_committee_shard.quorum_threshold()
                );
                return Ok(false);
            };

            // Wait for our own vote to make sure we've processed all transactions and we also have an up to date
            // database
            if votes.iter().all(|x| x.signature.public_key != our_vn.public_key) {
                warn!(target: LOG_TARGET, "❓️ Received enough votes but not our own vote for block {}", message.block_id);
                // return Ok(true);
            }

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

            signatures.sort_by(|a, b| a.public_key.cmp(&b.public_key));

            vote_data = VoteData {
                signatures,
                leaf_hashes,
                quorum_decision,
                block,
            };
        }

        // Generate merkle proofs for signer set
        let merged_proof = self
            .epoch_manager
            .get_validator_set_merged_merkle_proof(
                current_epoch,
                vote_data.signatures.iter().map(|s| s.public_key().clone()).collect(),
            )
            .await?;

        let block_height = vote_data.block.height();
        let qc = create_qc(vote_data, merged_proof);
        info!(target: LOG_TARGET, "🔥 New QC {}", qc);
        let high_qc = self.store.with_write_tx(|tx| qc.update_high_qc(tx))?;

        self.pacemaker.update_view(block_height, high_qc.block_height).await?;

        Ok(true)
    }

    fn calculate_threshold_decision(votes: &[Vote], local_committee_shard: &CommitteeShard) -> Option<QuorumDecision> {
        let mut count_accept = 0;
        let mut count_reject = 0;
        for vote in votes {
            match vote.decision {
                QuorumDecision::Accept => count_accept += 1,
                QuorumDecision::Reject => count_reject += 1,
            }
        }

        let threshold = local_committee_shard.quorum_threshold() as usize;
        if count_accept >= threshold {
            return Some(QuorumDecision::Accept);
        }
        if count_reject >= threshold {
            return Some(QuorumDecision::Reject);
        }

        None
    }

    fn validate_vote_message(&self, message: &VoteMessage, sender_leaf_hash: &FixedHash) -> Result<(), HotStuffError> {
        if !self.vote_signature_service.verify(
            &message.signature,
            sender_leaf_hash,
            &message.block_id,
            &message.decision,
        ) {
            return Err(HotStuffError::InvalidVoteSignature {
                signer_public_key: message.signature.public_key().to_string(),
            });
        }
        Ok(())
    }
}

fn create_qc(vote_data: VoteData, merged_proof: MergedValidatorNodeMerkleProof) -> QuorumCertificate {
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
        signatures,
        merged_proof,
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
