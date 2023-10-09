//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::ops::DerefMut;

use log::*;
use tari_common_types::types::FixedHash;
use tari_dan_common_types::{committee::CommitteeShard, hashing::MergedValidatorNodeMerkleProof, optional::Optional};
use tari_dan_storage::{
    consensus_models::{Block, HighQc, QuorumCertificate, QuorumDecision, Vote},
    StateStore,
    StateStoreWriteTransaction,
};
use tari_epoch_manager::EpochManagerReader;

use crate::{
    hotstuff::{error::HotStuffError, pacemaker_handle::PaceMakerHandle},
    messages::VoteMessage,
    traits::{ConsensusSpec, LeaderStrategy, VoteSignatureService},
};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::on_receive_vote";

pub struct OnReceiveVoteHandler<TConsensusSpec: ConsensusSpec> {
    store: TConsensusSpec::StateStore,
    leader_strategy: TConsensusSpec::LeaderStrategy,
    epoch_manager: TConsensusSpec::EpochManager,
    vote_signature_service: TConsensusSpec::VoteSignatureService,
    pacemaker: PaceMakerHandle,
}

impl<TConsensusSpec> OnReceiveVoteHandler<TConsensusSpec>
where TConsensusSpec: ConsensusSpec
{
    pub fn new(
        store: TConsensusSpec::StateStore,
        leader_strategy: TConsensusSpec::LeaderStrategy,
        epoch_manager: TConsensusSpec::EpochManager,
        vote_signature_service: TConsensusSpec::VoteSignatureService,
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

    #[allow(clippy::too_many_lines)]
    pub async fn handle(&self, message: VoteMessage<TConsensusSpec::Addr>) -> Result<(), HotStuffError> {
        debug!(
            target: LOG_TARGET,
            "🔥 Receive VOTE for node {} from {}", message.block_id, message.signature.public_key,
        );

        // Is a committee member sending us this vote?
        let committee = self.epoch_manager.get_local_committee(message.epoch).await?;
        if !committee.contains(&message.signature.public_key) {
            return Err(HotStuffError::ReceivedMessageFromNonCommitteeMember {
                epoch: message.epoch,
                sender: message.signature.public_key.to_string(),
                context: "OnReceiveVote".to_string(),
            });
        }

        // Are we the leader for the block being voted for?
        let vn = self.epoch_manager.get_our_validator_node(message.epoch).await?;

        let local_committee_shard = self.epoch_manager.get_local_committee_shard(message.epoch).await?;

        // Get the sender shard, and check that they are in the local committee
        let sender_vn = self
            .epoch_manager
            .get_validator_node(message.epoch, &message.signature.public_key)
            .await?;
        if !local_committee_shard.includes_shard(&sender_vn.shard_key) {
            return Err(HotStuffError::ReceivedMessageFromNonCommitteeMember {
                epoch: message.epoch,
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
                merkle_proof: message.merkle_proof,
            }
            .save(tx)?;

            let count = Vote::<TConsensusSpec::Addr>::count_for_block(tx.deref_mut(), &message.block_id)?;
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
            return Ok(());
        }
        let last_voted;
        let high_qc;
        {
            let mut tx = self.store.create_write_tx()?;
            let Some(block) = Block::get(tx.deref_mut(), &message.block_id).optional()? else {
                warn!(
                    target: LOG_TARGET,
                    "❌ Received {} votes for unknown block {}", count, message.block_id
                );
                tx.rollback()?;
                return Ok(());
            };

            if !self
                .leader_strategy
                .is_leader_for_next_block(&vn.address, &committee, block.height())
            {
                tx.rollback()?;
                return Err(HotStuffError::NotTheLeader {
                    details: format!(
                        "Not this leader for block {}, vote sent by {}",
                        message.block_id, vn.address
                    ),
                });
            }

            let h_qc = HighQc::get(tx.deref_mut())?;
            if h_qc.block_id == *block.id() {
                debug!(
                    target: LOG_TARGET,
                    "🔥 Received vote for block {} from {} ({} of {}), but we already have a QC for this block",
                    message.block_id,
                    from,
                    count,
                    local_committee_shard.quorum_threshold()
                );
                // We have already created a QC for this block
                tx.rollback()?;
                return Ok(());
            }

            let votes = block.get_votes(tx.deref_mut())?;
            let Some(quorum_decision) = Self::calculate_threshold_decision(&votes, &local_committee_shard) else {
                warn!(
                    target: LOG_TARGET,
                    "🔥 Received conflicting votes from replicas for block {} ({} of {}). Waiting for more votes.",
                    message.block_id,
                    count,
                    local_committee_shard.quorum_threshold()
                );
                tx.rollback()?;
                return Ok(());
            };

            // Wait for our own vote to make sure we've processed all transactions and we also have an up to date
            // database
            if votes.iter().all(|x| x.signature.public_key != vn.address) {
                warn!(target: LOG_TARGET, "❓️ Received enough votes but not our own vote for block {}", message.block_id);
                // tx.rollback()?;
                // return Ok(());
            }

            let signatures = votes.iter().map(|v| v.signature().clone()).collect::<Vec<_>>();
            let (leaf_hashes, proofs) = votes
                .iter()
                .map(|v| (v.sender_leaf_hash, v.merkle_proof.clone()))
                .unzip::<_, _, _, Vec<_>>();
            let merged_proof = MergedValidatorNodeMerkleProof::create_from_proofs(&proofs)?;

            let qc = QuorumCertificate::new(
                *block.id(),
                block.height(),
                block.epoch(),
                signatures,
                merged_proof,
                leaf_hashes,
                quorum_decision,
            );

            info!(target: LOG_TARGET, "🔥 New QC {}", qc);

            last_voted = block.as_last_voted();
            last_voted.set(&mut tx)?;
            high_qc = qc.update_high_qc(&mut tx)?;
            tx.commit()?;
        };
        self.pacemaker
            .reset_leader_timeout(last_voted.height(), high_qc.block_height)
            .await?;
        self.pacemaker.beat();

        Ok(())
    }

    fn calculate_threshold_decision(
        votes: &[Vote<TConsensusSpec::Addr>],
        local_committee_shard: &CommitteeShard,
    ) -> Option<QuorumDecision> {
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

    fn validate_vote_message(
        &self,
        message: &VoteMessage<TConsensusSpec::Addr>,
        sender_leaf_hash: &FixedHash,
    ) -> Result<(), HotStuffError> {
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
