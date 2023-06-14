//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::ops::DerefMut;

use log::*;
use tari_dan_common_types::hashing::MergedValidatorNodeMerkleProof;
use tari_dan_storage::{
    consensus_models::{Block, QuorumCertificate, Vote},
    StateStore,
    StateStoreWriteTransaction,
};

use crate::{
    hotstuff::{common::update_high_qc, error::HotStuffError},
    messages::VoteMessage,
    traits::{ConsensusSpec, EpochManager, LeaderStrategy},
};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::on_receive_vote";

pub struct OnReceiveVoteHandler<TConsensusSpec: ConsensusSpec> {
    store: TConsensusSpec::StateStore,
    leader_strategy: TConsensusSpec::LeaderStrategy,
    epoch_manager: TConsensusSpec::EpochManager,
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
    ) -> Self {
        Self {
            store,
            leader_strategy,
            epoch_manager,
        }
    }

    pub async fn handle(&self, from: TConsensusSpec::Addr, message: VoteMessage) -> Result<(), HotStuffError> {
        debug!(
            target: LOG_TARGET,
            "🔥 Receive VOTE for node {} from {}", message.block_id, from,
        );

        let addr = self.epoch_manager.get_our_validator_addr(message.epoch).await?;
        let committee = self.epoch_manager.get_local_committee(message.epoch).await?;
        if !committee.contains(&addr) {
            return Err(HotStuffError::ReceivedMessageFromNonCommitteeMember {
                epoch: message.epoch,
                sender: from.to_string(),
                context: "OnVoteReceived".to_string(),
            });
        }

        if !self.leader_strategy.is_leader(&addr, &committee, &message.block_id, 0) {
            return Err(HotStuffError::NotTheLeader {
                details: format!("Not this leader for block {}, vote sent by {}", message.block_id, addr),
            });
        }

        let local_commmittee_shard = self.epoch_manager.get_local_committee_shard(message.epoch).await?;

        // Get the sender shard, and check that they are in the local committee
        let sender_shard = self
            .epoch_manager
            .get_validator_shard(message.epoch, from.clone())
            .await?;
        if !local_commmittee_shard.includes_shard(&sender_shard) {
            return Err(HotStuffError::ReceivedMessageFromNonCommitteeMember {
                epoch: message.epoch,
                sender: from.to_string(),
                context: "OnVoteReceived".to_string(),
            });
        }

        let mut tx = self.store.create_write_tx()?;
        let block = Block::get(tx.deref_mut(), &message.block_id)?;
        if block.proposed_by() != local_commmittee_shard.our_shard_id() {
            return Err(HotStuffError::NotTheLeader {
                details: format!(
                    "Block {} was not proposed by this validator {}",
                    message.block_id,
                    local_commmittee_shard.our_shard_id()
                ),
            });
        }

        Vote {
            epoch: message.epoch,
            block_id: message.block_id,
            decision: message.decision,
            sender: sender_shard,
            signature: message.signature,
            merkle_proof: message.merkle_proof,
        }
        .save(&mut tx)?;

        let count = Vote::count_for_block(tx.deref_mut(), &message.block_id)?;

        if count < local_commmittee_shard.quorum_threshold() {
            info!(
                target: LOG_TARGET,
                "🔥 Received vote for block {} from {} ({} of {})",
                message.block_id,
                from,
                count,
                local_commmittee_shard.quorum_threshold()
            );
            return Ok(());
        }

        let votes = Vote::get_for_block(tx.deref_mut(), &message.block_id)?;

        let signatures = votes.iter().map(|v| v.signature().clone()).collect::<Vec<_>>();
        let leaf_hashes = votes.iter().map(|v| v.signature.create_challenge()).collect::<Vec<_>>();
        let proofs = votes.iter().map(|v| v.merkle_proof.clone()).collect();
        let merged_proof = MergedValidatorNodeMerkleProof::create_from_proofs(proofs)?;

        let qc = QuorumCertificate::new(
            *block.id(),
            block.height(),
            block.epoch(),
            0,
            signatures,
            merged_proof,
            leaf_hashes,
        );

        update_high_qc::<TConsensusSpec::StateStore>(&mut tx, &qc)?;

        tx.commit()?;

        Ok(())
    }
}
