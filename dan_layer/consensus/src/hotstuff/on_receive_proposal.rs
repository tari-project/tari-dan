//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use log::*;
use tari_dan_storage::consensus_models::ValidatorId;

use crate::{hotstuff::error::HotStuffError, messages::ProposalMessage, traits::ConsensusSpec};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::on_propose";

pub struct OnReceiveProposalHandler<TConsensnsSpec: ConsensusSpec> {
    _store: TConsensnsSpec::StateStore,
}

impl<TConsensnsSpec: ConsensusSpec> OnReceiveProposalHandler<TConsensnsSpec> {
    pub fn new(store: TConsensnsSpec::StateStore) -> Self {
        Self { _store: store }
    }

    pub async fn handle(&self, from: ValidatorId, message: ProposalMessage) -> Result<(), HotStuffError> {
        let block = message.block;
        debug!(
            target: LOG_TARGET,
            "🔥 Receive PROPOSAL for block {}, parent {}, height {} from {}",
            block.id(),
            block.parent(),
            block.height(),
            from,
        );
        todo!()

        // self.validate_proposed_block(&from, &block)?;
        //
        // let (involved_shards, last_vote_height, locked_block) = self.store.with_read_tx(|tx| {
        //     let last_voted = LastVoted::get(&tx, block.epoch())?;
        //     let locked_block = LockedBlock::get(&tx, block.epoch())?;
        //
        //     // let shards = Block::get_involved_shards(
        //     //     &tx,
        //     //     block
        //     //         .committed()
        //     //         .iter()
        //     //         .chain(block.precommitted())
        //     //         .chain(block.committed()),
        //     // )?;
        //
        //     Ok((vec![], last_voted, locked_block))
        // })?;
        //
        // // If we have not previously voted on this payload and the node extends the current locked node, then we vote
        // if last_vote_height.height.is_zero() ||
        //     block.height() > last_vote_height.height ||
        //     block.height() == last_vote_height.height &&
        //         (block.parent() == locked_block.block_id || block.height() > locked_block.height)
        // {
        //     let proposed_nodes = self.store.with_write_tx(|tx| {
        //         block.save(tx)?;
        //
        //         LeaderProposal {
        //             shard_id: block.shard_id,
        //             payload_id: block.payload_id,
        //             payload_height: block.payload_height,
        //             leader_round: block.leader_round,
        //             node: block.clone(),
        //         }
        //         .save(tx)?;
        //
        //         let proposals = LeaderProposal::get_many(tx, block.hash())?;
        //
        //         tx.save_leader_proposals(
        //             node.shard(),
        //             node.payload_id(),
        //             node.payload_height(),
        //             node.leader_round(),
        //             node.clone(),
        //         )?;
        //         tx.get_leader_proposals(node.payload_id(), node.payload_height(), &involved_shards)
        //     })?;
        //     // We group proposal by the shard id.
        //     let mut proposed_nodes_grouped_by_shard_id: HashMap<ShardId, Vec<HotStuffTreeNode<TAddr, TPayload>>> =
        //         HashMap::new();
        //     for proposed_node in proposed_nodes {
        //         proposed_nodes_grouped_by_shard_id
        //             .entry(proposed_node.shard())
        //             .or_default()
        //             .push(proposed_node);
        //     }
        //     // And now for each shard id we select only one proposal
        //     let mut proposed_nodes = Vec::new();
        //     for (_shard_id, nodes) in proposed_nodes_grouped_by_shard_id.drain() {
        //         proposed_nodes.push(nodes.into_iter().max_by_key(|node| node.leader_round()).unwrap());
        //     }
        //
        //     // Check the number of leader proposals for <shard, payload, node height>
        //     // i.e. all proposed nodes for the shards for the payload are on the same hotstuff phase (payload height)
        //     if proposed_nodes.len() < involved_shards.len() {
        //         info!(
        //             target: LOG_TARGET,
        //             "🔥 Waiting for more leader proposals ({}/{}) before voting on payload {}, height {}",
        //             proposed_nodes.len(),
        //             involved_shards.len(),
        //             payload_id,
        //             node.payload_height()
        //         );
        //
        //         self.update_nodes(&node)?;
        //         return Ok(());
        //     }
        //
        //     match self.decide_and_vote_on_all_nodes(payload, proposed_nodes).await {
        //         Ok(_) => {},
        //         Err(err @ HotStuffError::AllShardsRejected { .. }) => {
        //             self.publish_event(HotStuffEvent::Failed(payload_id, err.to_string()));
        //         },
        //         Err(err) => return Err(err),
        //     }
        //
        //     let mut tx = self.shard_store.create_write_tx()?;
        //     tx.set_last_voted_height(node.shard(), node.payload_id(), node.height(), node.leader_round())?;
        //     tx.commit()?;
        // } else {
        //     info!(
        //         target: LOG_TARGET,
        //         "🔥 Not ready to vote on payload {}, height {}, last_vote_height {}, locked_height {}",
        //         payload_id,
        //         node.height(),
        //         last_vote_height,
        //         locked_height
        //     );
        // }
        // self.update_nodes(&node)?;
        // // If all pledges for all shards and complete, then we can persist the payload changes
        // self.finalize_payload(&involved_shards, &node).await?;
        //
        // Ok(())
    }

    // fn validate_proposed_block(&self, _from: &ValidatorId, _block: &Block) -> Result<(), HotStuffError> {
    //     // TODO
    //     Ok(())
    // }
}
