//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use log::*;
use tari_dan_common_types::optional::Optional;
use tari_dan_storage::{
    consensus_models::{Block, LastSentVote, LeafBlock},
    StateStore,
};
use tokio::task;

use crate::{
    hotstuff::HotStuffError,
    messages::{HotstuffMessage, ProposalMessage, SyncRequestMessage},
    traits::{ConsensusSpec, OutboundMessaging},
};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::on_sync_request";

#[derive(Debug)]
pub struct OnSyncRequest<TConsensusSpec: ConsensusSpec> {
    store: TConsensusSpec::StateStore,
    outbound_messaging: TConsensusSpec::OutboundMessaging,
}

impl<TConsensusSpec: ConsensusSpec> OnSyncRequest<TConsensusSpec> {
    pub fn new(store: TConsensusSpec::StateStore, outbound_messaging: TConsensusSpec::OutboundMessaging) -> Self {
        Self {
            store,
            outbound_messaging,
        }
    }

    pub fn handle(&self, from: TConsensusSpec::Addr, msg: SyncRequestMessage) {
        let mut outbound_messaging = self.outbound_messaging.clone();
        let store = self.store.clone();

        task::spawn(async move {
            let result = store.with_read_tx(|tx| {
                let leaf_block = LeafBlock::get(tx)?;

                if leaf_block.height() < msg.high_qc.block_height() {
                    return Err(HotStuffError::InvalidSyncRequest {
                        details: format!(
                            "Received catch up request from {} for block {} but our leaf block is {}. Ignoring \
                             request.",
                            from, msg.high_qc, leaf_block
                        ),
                    });
                }

                info!(
                    target: LOG_TARGET,
                    "🌐 Received catch up request from {} from block {} to {}",
                    from,
                    msg.high_qc,
                    leaf_block
                );
                let blocks = Block::get_all_blocks_between(tx, msg.high_qc.block_id(), leaf_block.block_id(), true)?;

                info!(
                    target: LOG_TARGET,
                    "🌐 Sending {} blocks to {}",
                    blocks.len(),
                    from
                );

                Ok::<_, HotStuffError>(blocks)
            });

            let blocks = match result {
                Ok(blocks) => blocks,
                Err(err) => {
                    warn!(target: LOG_TARGET, "Failed to fetch blocks for sync request: {}", err);
                    return;
                },
            };

            for block in blocks {
                debug!(
                    target: LOG_TARGET,
                    "🌐 Sending block {} to {}",
                    block,
                    from
                );
                if let Err(err) = outbound_messaging
                    .send(from.clone(), HotstuffMessage::Proposal(ProposalMessage { block }))
                    .await
                {
                    warn!(target: LOG_TARGET, "Error sending SyncResponse: {err}");
                    return;
                }
            }

            // Send last vote. TODO: This isn't quite
            let maybe_last_vote = match store.with_read_tx(|tx| LastSentVote::get(tx)).optional() {
                Ok(last_vote) => last_vote,
                Err(err) => {
                    warn!(target: LOG_TARGET, "Failed to fetch last vote for sync request: {}", err);
                    return;
                },
            };
            if let Some(last_vote) = maybe_last_vote {
                if let Err(err) = outbound_messaging
                    .send(from.clone(), HotstuffMessage::Vote(last_vote.into()))
                    .await
                {
                    warn!(target: LOG_TARGET, "Leader channel closed while sending LastVote {err}");
                }
            }

            // let _ignore = outbound_messaging
            //     .send((
            //         from,
            //         HotstuffMessage::SyncResponse(SyncResponseMessage {
            //             epoch: msg.epoch,
            //             blocks,
            //         }),
            //     ))
            //     .await;
        });
    }
}
