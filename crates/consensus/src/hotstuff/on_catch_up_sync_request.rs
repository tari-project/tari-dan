//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::time::Duration;

use log::*;
use tari_consensus_types::{LastProposed, LeafBlock};
use tari_ootle_common_types::{Epoch, NodeHeight, optional::Optional};
use tari_ootle_storage::{
    StateStore,
    consensus_models::{Block, BookkeepingModel},
};

use crate::{
    bounded_spawn::PerKeyBoundedSpawn,
    hotstuff::HotStuffError,
    messages::{CatchUpRequestMessage, HotstuffMessage, ProposalMessage},
    traits::{ConsensusSpec, OutboundMessaging},
};

const LOG_TARGET: &str = "tari::ootle::consensus::hotstuff::on_sync_request";

/// Number of catch-up requests served concurrently. Each one walks the block store and streams every block up
/// to our leaf, so the work this node does must not scale with the number of peers asking.
const MAX_CONCURRENT_SYNC_REQUESTS: usize = 5;
/// A peer only ever needs one response at a time, and holding it to one keeps the rest of the pool for its committee.
const MAX_SYNC_REQUESTS_PER_PEER: usize = 1;
/// A response streams every block up to our leaf, so it is allowed to be slow. Past this the peer is not keeping up
/// with what it asked for, and its permit is worth more to someone else.
const SYNC_RESPONSE_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug)]
pub struct OnSyncRequest<TConsensusSpec: ConsensusSpec> {
    store: TConsensusSpec::StateStore,
    outbound_messaging: TConsensusSpec::OutboundMessaging,
    bounded_spawner: PerKeyBoundedSpawn<TConsensusSpec::Addr>,
}

impl<TConsensusSpec: ConsensusSpec> OnSyncRequest<TConsensusSpec> {
    pub fn new(store: TConsensusSpec::StateStore, outbound_messaging: TConsensusSpec::OutboundMessaging) -> Self {
        Self {
            store,
            outbound_messaging,
            bounded_spawner: PerKeyBoundedSpawn::new(
                MAX_CONCURRENT_SYNC_REQUESTS,
                MAX_SYNC_REQUESTS_PER_PEER,
                SYNC_RESPONSE_TIMEOUT,
            ),
        }
    }

    pub fn handle(&self, from: TConsensusSpec::Addr, epoch: Epoch, msg: CatchUpRequestMessage) {
        if msg.epoch != epoch {
            warn!(
                target: LOG_TARGET,
                "Received SyncRequest from {} for epoch {} but our epoch is {}. Ignoring request.",
                from,
                msg.epoch,
                epoch
            );
            return;
        }

        let outbound_messaging = self.outbound_messaging.clone();
        let store = self.store.clone();

        if self
            .bounded_spawner
            .try_spawn(from.clone(), {
                let from = from.clone();
                Self::handle_request_task(store, outbound_messaging, from, epoch, msg)
            })
            .is_err()
        {
            warn!(
                target: LOG_TARGET,
                "⚠️ No catch-up request slot available for {}, dropping the request",
                from
            );
        }
    }

    #[allow(clippy::too_many_lines)]
    async fn handle_request_task(
        store: TConsensusSpec::StateStore,
        mut outbound_messaging: TConsensusSpec::OutboundMessaging,
        from: TConsensusSpec::Addr,
        epoch: Epoch,
        msg: CatchUpRequestMessage,
    ) {
        let result = store.with_read_tx(|tx| {
            let mut leaf_block = LeafBlock::get(tx, epoch)?;
            // Include the block we last proposed if applicable.
            if let Some(last_proposed) = LastProposed::get(tx, epoch).optional()? &&
                last_proposed.epoch == leaf_block.epoch() &&
                last_proposed.height > leaf_block.height()
            {
                leaf_block = last_proposed.as_leaf_block();
            }

            if leaf_block.epoch() != msg.epoch {
                info!(
                    target: LOG_TARGET,
                    "Received catch up request from {} for epoch {} but our leaf block is {}. Ignoring request.",
                    from,
                    msg.epoch,
                    leaf_block
                );
                return Ok(None);
            }

            if leaf_block.height.is_zero() {
                info!(
                    target: LOG_TARGET,
                    "This node is at height 0 so cannot return any sync blocks. Ignoring request"
                );
                return Ok(None);
            }

            if leaf_block.height() < msg.block_height {
                return Err(HotStuffError::InvalidSyncRequest {
                    details: format!(
                        "Received catch up request from {} for block {} but our leaf block is {}. Ignoring request.",
                        from, msg.block_height, leaf_block
                    ),
                });
            }

            info!(
                target: LOG_TARGET,
                "🌐 Received catch up request from {} from block {} to {}",
                from,
                msg.block_height,
                leaf_block
            );
            Ok(Some(leaf_block))
        });

        let leaf_block = match result {
            Ok(Some(leaf_block)) => leaf_block,
            Ok(None) => {
                return;
            },
            Err(err) => {
                warn!(target: LOG_TARGET, "Failed to process sync request: {}", err);
                return;
            },
        };

        let mut start_height = msg.block_height.max(NodeHeight(1));
        while start_height < leaf_block.height() {
            let result = store.with_read_tx(|tx| {
                Block::get_all_blocks_between(tx, msg.epoch, start_height, leaf_block.height(), false, 100)
            });

            let blocks = match result {
                Ok(blocks) => blocks,
                Err(err) => {
                    warn!(target: LOG_TARGET, "Failed to fetch blocks for catch-up request: {}", err);
                    return;
                },
            };

            if blocks.is_empty() {
                warn!(
                    target: LOG_TARGET,
                    "No blocks found between heights {} and {} for epoch {}",
                    start_height,
                    leaf_block.height(),
                    epoch
                );
                return;
            }
            start_height = blocks
                .last()
                .map(|b| b.height() + NodeHeight(1))
                .unwrap_or(leaf_block.height());

            info!(
                target: LOG_TARGET,
                "🌐 Sending {} block(s) ({} to {}) to {}",
                blocks.len(),
                blocks.first().map(|b| b.height()).unwrap_or_default(),
                blocks.last().map(|b| b.height()).unwrap_or_default(),
                from
            );

            for block in blocks {
                // TODO(perf): O(n) queries
                let foreign_proposals = match store.with_read_tx(|tx| block.get_foreign_proposals(tx)) {
                    Ok(foreign_proposals) => foreign_proposals,
                    Err(err) => {
                        warn!(target: LOG_TARGET, "Failed to fetch foreign proposals for block {}: {}", block, err);
                        return;
                    },
                };

                debug!(
                    target: LOG_TARGET,
                    "🌐 Sending block {} to {}",
                    block,
                    from
                );

                let is_tip = block.height() == leaf_block.height();
                let proposal = ProposalMessage {
                    block,
                    foreign_proposals: foreign_proposals.into_iter().map(|p| p.into_proposal()).collect(),
                };
                // The requester does not vote on catch-up blocks, because a later block certifies each of them. Our
                // tip has no certificate yet, so it goes as a proposal: the requester votes on it if it is still
                // current when it gets there.
                let message = if is_tip {
                    HotstuffMessage::new_proposal(proposal)
                } else {
                    HotstuffMessage::new_catch_up_sync_response(proposal)
                };

                if let Err(err) = outbound_messaging.send(from.clone(), message).await {
                    warn!(target: LOG_TARGET, "Error sending SyncResponse: {err}");
                    return;
                }
            }
        }
    }
}
