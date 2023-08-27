//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use std::ops::DerefMut;

use log::*;
use tari_dan_common_types::{Epoch, NodeHeight};
use tari_dan_storage::{
    consensus_models::{Block, HighQc},
    StateStore,
};
use tari_epoch_manager::EpochManagerReader;
use tokio::sync::mpsc;

use crate::{
    hotstuff::HotStuffError,
    messages::{HotstuffMessage, NewViewMessage},
    traits::{ConsensusSpec, LeaderStrategy},
};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::on_next_sync_view";

pub struct OnNextSyncViewHandler<TConsensusSpec: ConsensusSpec> {
    store: TConsensusSpec::StateStore,
    tx_leader: mpsc::Sender<(TConsensusSpec::Addr, HotstuffMessage<TConsensusSpec::Addr>)>,
    leader_strategy: TConsensusSpec::LeaderStrategy,
    epoch_manager: TConsensusSpec::EpochManager,
}

impl<TConsensusSpec: ConsensusSpec> OnNextSyncViewHandler<TConsensusSpec> {
    pub fn new(
        store: TConsensusSpec::StateStore,
        tx_leader: mpsc::Sender<(TConsensusSpec::Addr, HotstuffMessage<TConsensusSpec::Addr>)>,
        leader_strategy: TConsensusSpec::LeaderStrategy,
        epoch_manager: TConsensusSpec::EpochManager,
    ) -> Self {
        Self {
            store,
            tx_leader,
            leader_strategy,
            epoch_manager,
        }
    }

    pub async fn handle(&mut self, epoch: Epoch, new_height: NodeHeight) -> Result<(), HotStuffError> {
        info!(target: LOG_TARGET, "⚠️ Leader failure: NEXTSYNCVIEW for epoch {} and node height {}", epoch, new_height);
        let local_committee = self.epoch_manager.get_local_committee(epoch).await?;
        let current_epoch = self.epoch_manager.current_epoch().await?;

        let high_qc = self.store.with_write_tx(|tx| {
            let high_qc = HighQc::get(tx.deref_mut())?.get_quorum_certificate(tx.deref_mut())?;
            let mut current_height = high_qc.block_height() + NodeHeight(1);
            let mut parent_block_id = *high_qc.block_id();
            while current_height <= new_height {
                let leader = self.leader_strategy.get_leader(&local_committee, current_height);
                let dummy_block = Block::dummy_block(
                    parent_block_id,
                    leader.clone(),
                    current_height,
                    high_qc.clone(),
                    current_epoch,
                );
                debug!(target: LOG_TARGET, "🍼 DUMMY BLOCK: {}. Leader: {}", dummy_block, leader);
                if dummy_block.save(tx)? {
                    dummy_block.as_leaf_block().set(tx)?;
                }
                current_height += NodeHeight(1);
                parent_block_id = *dummy_block.id();
            }

            Ok::<_, HotStuffError>(high_qc)
        })?;

        let next_leader = self
            .leader_strategy
            .get_leader_for_next_block(&local_committee, new_height);

        let message = NewViewMessage {
            high_qc,
            new_height,
            epoch: current_epoch,
        };

        info!(target: LOG_TARGET, "🔥 Send NEWVIEW ({new_height}) to {next_leader}");

        self.tx_leader
            .send((next_leader.clone(), HotstuffMessage::NewView(message)))
            .await
            .map_err(|_| HotStuffError::InternalChannelClosed {
                context: "tx_leader in OnNextSyncViewHandler::send_to_leader",
            })
    }
}
