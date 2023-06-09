//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use log::*;

use crate::{hotstuff::error::HotStuffError, messages::VoteMessage, traits::ConsensusSpec};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::on_receive_vote";

pub struct OnReceiveVoteHandler<TConsensusSpec: ConsensusSpec> {
    _store: TConsensusSpec::StateStore,
}

impl<TConsensusSpec: ConsensusSpec> OnReceiveVoteHandler<TConsensusSpec> {
    pub fn new(store: TConsensusSpec::StateStore) -> Self {
        Self { _store: store }
    }

    pub async fn handle(&self, from: TConsensusSpec::Addr, message: VoteMessage) -> Result<(), HotStuffError> {
        debug!(
            target: LOG_TARGET,
            "🔥 Receive VOTE for node {} from {}", message.block_id, from,
        );
        todo!()
        // let mut on_propose = None;
        // let node;
        // {
        //     let mut tx = self.shard_store.create_read_tx()?;
        //     // Avoid duplicates
        //     if tx.has_vote_for(&from, msg.local_node_hash())? {
        //         println!("🔥 Vote with node hash {} already received", msg.local_node_hash());
        //         return Ok(());
        //     }
        //
        //     node = tx
        //         .get_node(&msg.local_node_hash())
        //         .optional()?
        //         .ok_or(HotStuffError::InvalidVote(format!(
        //             "Node with hash {} not found",
        //             msg.local_node_hash()
        //         )))?;
        //     if *node.proposed_by() != self.public_key {
        //         return Err(HotStuffError::NotTheLeader);
        //     }
        // }
        //
        // let valid_committee = self.epoch_manager.get_committee(node.epoch(), node.shard()).await?;
        // {
        //     if !valid_committee.contains(&from) {
        //         return Err(HotStuffError::ReceivedMessageFromNonCommitteeMember);
        //     }
        //     let mut tx = self.shard_store.create_write_tx()?;
        //
        //     // Collect votes
        //     tx.save_received_vote_for(from, msg.local_node_hash(), msg.clone())?;
        //
        //     let votes: Vec<VoteMessage> = tx.get_received_votes_for(msg.local_node_hash())?;
        //
        //     if votes.len() == valid_committee.consensus_threshold() {
        //         let validator_metadata = votes.iter().map(|v| v.validator_metadata().clone()).collect();
        //         let proofs = votes
        //             .iter()
        //             .map(|v| v.merkle_proof().unwrap().clone())
        //             .collect::<Vec<_>>();
        //
        //         let merged_proof = MergedBalancedBinaryMerkleProof::create_from_proofs(proofs).unwrap();
        //         let leaf_hashes = votes.iter().map(|v| v.node_hash()).collect::<Vec<_>>();
        //
        //         // TODO: Check all votes
        //         let main_vote = votes.get(0).unwrap();
        //
        //         let qc = QuorumCertificate::new(
        //             node.payload_id(),
        //             node.payload_height(),
        //             *node.hash(),
        //             node.height(),
        //             node.shard(),
        //             node.epoch(),
        //             self.public_key.clone(),
        //             main_vote.decision(),
        //             main_vote.all_shard_pledges().clone(),
        //             validator_metadata,
        //             Some(merged_proof),
        //             leaf_hashes,
        //         );
        //         self.update_high_qc(&mut tx, node.proposed_by().clone(), qc)?;
        //
        //         on_propose = Some((node.shard(), node.payload_id()));
        //     }
        //
        //     // commit the transaction
        //     tx.commit()?;
        // }
        //
        // // Propose the next node
        // if let Some((shard_id, payload_id)) = on_propose {
        //     // TODO: This should go in a some component that controls message flows and events
        //     let epoch = self.epoch_manager.current_epoch().await?;
        //     let committee = self.epoch_manager.get_committee(epoch, shard_id).await?;
        //     if committee.is_empty() {
        //         return Err(HotStuffError::NoCommitteeForShard { shard: shard_id, epoch });
        //     }
        //     if self.is_leader(payload_id, shard_id, &committee)? {
        //         self.leader_on_propose(shard_id, payload_id).await?;
        //     }
        // }
        // Ok(())
    }
}
