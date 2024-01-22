//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use log::*;
use tari_dan_common_types::optional::Optional;
use tari_dan_storage::{
    consensus_models::{Block, LastSentVote, LastVoted},
    StateStore,
};
use tokio::{sync::mpsc, task};

use crate::{
    hotstuff::HotStuffError,
    messages::{HotstuffMessage, ProposalMessage, SyncRequestMessage},
    traits::ConsensusSpec,
};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::on_sync_request";

pub(super) const MAX_BLOCKS_PER_SYNC: usize = 100;

#[derive(Debug)]
pub struct OnSyncRequest<TConsensusSpec: ConsensusSpec> {
    store: TConsensusSpec::StateStore,
    tx_leader: mpsc::Sender<(TConsensusSpec::Addr, HotstuffMessage)>,
}

impl<TConsensusSpec: ConsensusSpec> OnSyncRequest<TConsensusSpec> {
    pub fn new(
        store: TConsensusSpec::StateStore,
        tx_leader: mpsc::Sender<(TConsensusSpec::Addr, HotstuffMessage)>,
    ) -> Self {
        Self { store, tx_leader }
    }

    pub fn handle(&self, from: TConsensusSpec::Addr, msg: SyncRequestMessage) {
        let tx_leader = self.tx_leader.clone();
        let store = self.store.clone();

        task::spawn(async move {
            let result = store.with_read_tx(|tx| {
                let last_voted = LastVoted::get(tx)?;

                info!(
                    target: LOG_TARGET,
                    "🌐 Received catch up request from {} from block {} to {}",
                    from,
                    msg.high_qc,
                    last_voted
                );
                let blocks = Block::get_all_blocks_between(tx, msg.high_qc.block_id(), last_voted.block_id(), false)?;

                debug!(
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

            for block in blocks.into_iter().take(MAX_BLOCKS_PER_SYNC) {
                debug!(
                    target: LOG_TARGET,
                    "🌐 Sending block {} to {}",
                    block,
                    from
                );
                if tx_leader
                    .send((from.clone(), HotstuffMessage::Proposal(ProposalMessage { block })))
                    .await
                    .is_err()
                {
                    warn!(target: LOG_TARGET, "Leader channel closed while sending SyncResponse");
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
                if tx_leader
                    .send((from.clone(), HotstuffMessage::Vote(last_vote.into())))
                    .await
                    .is_err()
                {
                    warn!(target: LOG_TARGET, "Leader channel closed while sending LastVote");
                }
            }

            // let _ignore = tx_leader
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
