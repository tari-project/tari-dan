//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::ops::DerefMut;

use log::*;
use tari_common_types::types::FixedHash;
use tari_dan_common_types::hashing::MergedValidatorNodeMerkleProof;
use tari_dan_storage::{
    consensus_models::{Block, QuorumCertificate, Vote},
    StateStore,
};

use crate::{
    hotstuff::{common::update_high_qc, error::HotStuffError, on_beat::OnBeat},
    messages::VoteMessage,
    traits::{ConsensusSpec, EpochManager, LeaderStrategy, VoteSignatureService},
};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::on_receive_vote";

pub struct OnReceiveVoteHandler<TConsensusSpec: ConsensusSpec> {
    store: TConsensusSpec::StateStore,
    leader_strategy: TConsensusSpec::LeaderStrategy,
    epoch_manager: TConsensusSpec::EpochManager,
    vote_signature_service: TConsensusSpec::VoteSignatureService,
    on_beat: OnBeat,
}

impl<TConsensusSpec> OnReceiveVoteHandler<TConsensusSpec>
where
    TConsensusSpec: ConsensusSpec,
    HotStuffError: From<<TConsensusSpec::EpochManager as EpochManager>::Error>,
{
    pub fn new(
        store: TConsensusSpec::StateStore,
        leader_strategy: TConsensusSpec::LeaderStrategy,
        epoch_manager: TConsensusSpec::EpochManager,
        vote_signature_service: TConsensusSpec::VoteSignatureService,
        on_beat: OnBeat,
    ) -> Self {
        Self {
            store,
            leader_strategy,
            epoch_manager,
            on_beat,
            vote_signature_service,
        }
    }

    pub async fn handle(&self, from: TConsensusSpec::Addr, message: VoteMessage) -> Result<(), HotStuffError> {
        debug!(
            target: LOG_TARGET,
            "🔥 Receive VOTE for node {} from {}", message.block_id, from,
        );

        // Is a committee member sending us this vote?
        let committee = self.epoch_manager.get_local_committee(message.epoch).await?;
        if !committee.contains(&from) {
            return Err(HotStuffError::ReceivedMessageFromNonCommitteeMember {
                epoch: message.epoch,
                sender: from.to_string(),
                context: "OnReceiveVote".to_string(),
            });
        }

        // Are we the leader for the block being voted for?
        let addr = self.epoch_manager.get_our_validator_addr(message.epoch).await?;
        if !self.leader_strategy.is_leader(&addr, &committee, &message.block_id, 0) {
            return Err(HotStuffError::NotTheLeader {
                details: format!("Not this leader for block {}, vote sent by {}", message.block_id, addr),
            });
        }

        let local_committee_shard = self.epoch_manager.get_local_committee_shard(message.epoch).await?;

        // Get the sender shard, and check that they are in the local committee
        let sender_shard_id = self.epoch_manager.get_validator_shard(message.epoch, &from).await?;
        if !local_committee_shard.includes_shard(&sender_shard_id) {
            return Err(HotStuffError::ReceivedMessageFromNonCommitteeMember {
                epoch: message.epoch,
                sender: from.to_string(),
                context: "OnReceiveVote".to_string(),
            });
        }

        let sender_leaf_hash = self.epoch_manager.get_validator_leaf_hash(message.epoch, &from).await?;

        self.validate_vote_message(&message, &sender_leaf_hash)?;

        let our_validator_shard_id = self.epoch_manager.get_our_validator_shard(message.epoch).await?;

        let (block, count) = self.store.with_write_tx(|tx| {
            let block = Block::get(tx.deref_mut(), &message.block_id)?;
            if *block.proposed_by() != our_validator_shard_id {
                return Err(HotStuffError::NotTheLeader {
                    details: format!(
                        "Block {} was not proposed by this validator {}",
                        message.block_id, our_validator_shard_id
                    ),
                });
            }

            Vote {
                epoch: message.epoch,
                block_id: message.block_id,
                decision: message.decision,
                sender_leaf_hash,
                signature: message.signature,
                merkle_proof: message.merkle_proof,
            }
            .save(tx)?;

            let count = Vote::count_for_block(tx.deref_mut(), &message.block_id)?;
            Ok::<_, HotStuffError>((block, count))
        })?;

        // We only generate the next high qc once when we have a quorum of votes. Any subsequent votes are not included
        // in the QC.
        if count != local_committee_shard.quorum_threshold() as usize {
            info!(
                target: LOG_TARGET,
                "🔥 Received vote for block {} from {} ({} of {})",
                message.block_id,
                from,
                count,
                local_committee_shard.quorum_threshold()
            );
            return Ok(());
        }

        self.store.with_write_tx(|tx| {
            let votes = block.get_votes(tx.deref_mut())?;

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
                0,
                signatures,
                merged_proof,
                leaf_hashes,
            );

            update_high_qc::<TConsensusSpec::StateStore>(tx, &qc)
        })?;

        self.on_beat.beat();

        Ok(())
    }

    fn validate_vote_message(&self, message: &VoteMessage, sender_leaf_hash: &FixedHash) -> Result<(), HotStuffError> {
        let challenge =
            self.vote_signature_service
                .create_challenge(sender_leaf_hash, &message.block_id, &message.decision);
        if !message.signature.verify(challenge) {
            return Err(HotStuffError::InvalidVoteSignature {
                signer_public_key: message.signature.public_key().clone(),
            });
        }
        Ok(())
    }
}
