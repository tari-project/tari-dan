//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::{BTreeSet, HashSet};

use log::{debug, info};
use tari_dan_common_types::{committee::Committee, shard_bucket::ShardBucket};
use tari_dan_storage::{
    consensus_models::{Block, Command, ExecutedTransaction},
    StateStore,
    StateStoreReadTransaction,
};
use tari_epoch_manager::EpochManagerReader;
use tokio::sync::mpsc;

use super::{common::CommitteeAndMessage, HotStuffError};
use crate::{
    messages::{HotstuffMessage, ProposalMessage},
    traits::ConsensusSpec,
};

#[derive(Clone)]
pub struct Proposer<TConsensusSpec: ConsensusSpec> {
    store: TConsensusSpec::StateStore,
    epoch_manager: TConsensusSpec::EpochManager,
    tx_broadcast: mpsc::Sender<CommitteeAndMessage<TConsensusSpec::Addr>>,
}

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::on_propose_foreignly";

impl<TConsensusSpec> Proposer<TConsensusSpec>
where TConsensusSpec: ConsensusSpec
{
    pub fn new(
        store: TConsensusSpec::StateStore,
        epoch_manager: TConsensusSpec::EpochManager,
        tx_broadcast: mpsc::Sender<CommitteeAndMessage<TConsensusSpec::Addr>>,
    ) -> Self {
        Self {
            store,
            epoch_manager,
            tx_broadcast,
        }
    }

    pub async fn broadcast_proposal_foreignly(&self, block: &Block<TConsensusSpec::Addr>) -> Result<(), HotStuffError> {
        let num_committees = self.epoch_manager.get_num_committees(block.epoch()).await?;

        let validator = self.epoch_manager.get_our_validator_node(block.epoch()).await?;
        let local_bucket = validator.shard_key.to_committee_bucket(num_committees);
        let non_local_buckets = self
            .store
            .with_read_tx(|tx| get_non_local_buckets(tx, block, num_committees, local_bucket))?;
        info!(
            target: LOG_TARGET,
            "🌿 PROPOSING foreignly new locked block {} to {} foreign shards. justify: {} ({}), parent: {}",
            block,
            non_local_buckets.len(),
            block.justify().block_id(),
            block.justify().block_height(),
            block.parent()
        );
        debug!(
            target: LOG_TARGET,
            "non_local_buckets : [{}]",
            non_local_buckets.iter().map(|s|s.to_string()).collect::<Vec<_>>().join(","),
        );
        let non_local_committees = self
            .epoch_manager
            .get_committees_by_buckets(block.epoch(), non_local_buckets)
            .await?;
        info!(
            target: LOG_TARGET,
            "🌿 Broadcasting new locked block {} to {} foreign committees.",
            block,
            non_local_committees.len(),
        );
        let local_committee = self.epoch_manager.get_local_committee(block.epoch()).await?;
        let my_index = local_committee
            .members
            .iter()
            .position(|member| *member == validator.address)
            .expect("I should be part of my local committee");

        let mut foreign_nodes = Vec::new();
        for non_local_committee in non_local_committees.values() {
            // Get the indices in foreign committee whom I should send the block. We shift the index for some
            // deterministic randomness. Otherwise the same nodes will be senders all the time.
            let send_to = whom_should_i_resend_the_block(
                (my_index + block.height().as_u64() as usize) % local_committee.len(),
                local_committee.len(),
                non_local_committee.len(),
            );
            debug!(target:LOG_TARGET, "I should send the block to {:?}", send_to);
            // Adding the block.height is to make it little bit random, otherwise we will always send it to the same
            // nodes. It has to be deterministic, so all the nodes in the committee agree on the same nodes.
            foreign_nodes.extend(send_to.into_iter().map(|index| {
                non_local_committee.members[(index + block.height().as_u64() as usize) % non_local_committee.len()]
                    .clone()
            }))
        }
        // If the foreign_nodes is empty, that means we are not reponsible for distributing the block to the foreign
        // committees
        if !foreign_nodes.is_empty() {
            // foreign_nodes holds all the nodes in the foreign committees that we should send the block to, there can
            // be nodes from more than one committee, or it can be nodes only from single committee
            self.tx_broadcast
                .send((
                    Committee::new(foreign_nodes),
                    HotstuffMessage::ForeignProposal(ProposalMessage { block: block.clone() }),
                ))
                .await
                .map_err(|_| HotStuffError::InternalChannelClosed {
                    context: "proposing locked block to foreing committees",
                })?;
        }

        Ok(())
    }
}

pub fn get_non_local_buckets<TTx: StateStoreReadTransaction>(
    tx: &mut TTx,
    block: &Block<TTx::Addr>,
    num_committees: u32,
    local_bucket: ShardBucket,
) -> Result<HashSet<ShardBucket>, HotStuffError> {
    get_non_local_buckets_from_commands(tx, block.commands(), num_committees, local_bucket)
}

pub fn get_non_local_buckets_from_commands<TTx: StateStoreReadTransaction>(
    tx: &mut TTx,
    commands: &BTreeSet<Command>,
    num_committees: u32,
    local_bucket: ShardBucket,
) -> Result<HashSet<ShardBucket>, HotStuffError> {
    let prepared_iter = commands.iter().filter_map(|cmd| cmd.local_prepared()).map(|t| &t.id);
    let prepared_txs = ExecutedTransaction::get_involved_shards(tx, prepared_iter)?;
    let non_local_buckets = prepared_txs
        .into_iter()
        .flat_map(|(_, shards)| shards)
        .map(|shard| shard.to_committee_bucket(num_committees))
        .filter(|bucket| *bucket != local_bucket)
        .collect();
    Ok(non_local_buckets)
}

// Returns the indices in foreign committee whom I should send the block, so we guarantee that at least one
// honest node receives it in the foreign committee. So we need to send it to at least f+1 node in the foreign
// committee. But also take into consideration that our committee can have f dishonest nodes.
fn whom_should_i_resend_the_block(
    my_index: usize,
    my_committee_size: usize,
    foreign_committee_size: usize,
) -> Vec<usize> {
    if foreign_committee_size / 3 + my_committee_size / 3 < my_committee_size {
        // We can do n to 1 mapping, but am I one that should be sending it?
        if foreign_committee_size / 3 + my_committee_size / 3 + 1 > my_index {
            vec![my_index % foreign_committee_size]
        } else {
            vec![]
        }
    } else {
        // We can't do n to 1 mapping, the foreign committee is too big (more than 2 times), so now we have need
        // 1 to N mapping.
        // Lets the size of the committee be the 3f+1 and the foreign committee 3g+1.
        // We know that f+g+1 > 3f+1 (that's above)
        // So now we need to send more than 1 message per node. If we send n messages per node from x nodes, then we
        // need (x-f)*n > g, because f nodes can be faulty, and we need to hit at least one honest node. So the
        // smallest n is n=g/(x-f)+1. If we send it from the whole committee then n=g/(2f+1)+1. Ok but now, due to
        // rounding, we don't have to send it from all the nodes. We just need to satify the (x-f)*n>g. So now we
        // compute the x, x=g/n+f+1.
        let my_f = (my_committee_size - 1) / 3;
        let foreign_f = (foreign_committee_size - 1) / 3;
        let n = foreign_f / (my_committee_size - my_f) + 1;
        let nodes = foreign_f / n + 1 + my_f;
        // So now we have 1 to N mapping, nodes will each send n messages
        if my_index < nodes {
            ((my_index * n)..((my_index + 1) * n))
                .map(|i| i % foreign_committee_size)
                .collect()
        } else {
            vec![]
        }
    }
}
#[cfg(test)]
mod test {
    use super::whom_should_i_resend_the_block;

    #[test]
    fn test_whom_should_i_resend_the_block() {
        let to = whom_should_i_resend_the_block(0, 1, 1);
        // If we are the only one and there is only one person in the other committee, we have to send it.
        assert_eq!(to, vec![0]);
        let to = whom_should_i_resend_the_block(0, 1, 7);
        // If we are the only one and there are 7 people in the other committee, we have to send it, but not to all,
        // just f+1 (so at least one honest node receives it)
        assert_eq!(to, vec![0, 1, 2]);
        for index in 0..3 {
            // We need to make sure that at least one honest node sends it, so we select f+1 nodes to send it.
            let to = whom_should_i_resend_the_block(index, 7, 1);
            assert_eq!(to, vec![0]);
        }
        for index in 3..7 {
            // We already selected f+1 nodes.
            let to = whom_should_i_resend_the_block(index, 7, 1);
            assert!(to.is_empty());
        }
        for index in 0..5 {
            // If we have committees of equal sizes we need to guarantee that at least one honest node receives it. So
            // we select 2f+1 nodes to send to 2f+1 nodes. If we have f dishonest nodes, then only f+1 messages go
            // through. And if we have f dishonest nodes in the other committee, then at least one honest node receives
            // it.
            let to = whom_should_i_resend_the_block(index, 7, 7);
            assert_eq!(to, vec![index]);
        }
        for index in 5..7 {
            let to = whom_should_i_resend_the_block(index, 7, 7);
            assert!(to.is_empty());
        }
        for index in 0..4 {
            // If we are much smaller committee, we need to send more messages per node.
            let to = whom_should_i_resend_the_block(index, 4, 19);
            assert_eq!(to, vec![index * 3, index * 3 + 1, index * 3 + 2]);
        }
    }
}
