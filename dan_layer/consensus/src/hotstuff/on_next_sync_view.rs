//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use log::info;
use tari_dan_common_types::{Epoch, NodeHeight};
use tari_dan_storage::{
    consensus_models::{HighQc, LeafBlock},
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
        info!(target: LOG_TARGET, "🔥 Handle NEXTSYNCVIEW for epoch {} and node height {}", epoch, new_height);
        // let leaf_block = self.store.with_read_tx(|tx| LeafBlock::get(tx, epoch))?;
        let local_committee = self.epoch_manager.get_local_committee(epoch).await?;
        let next_leader = self.leader_strategy.get_leader(&local_committee, new_height);

        let high_qc = self
            .store
            .with_read_tx(|tx| HighQc::get(tx, epoch).and_then(|qc| qc.get_quorum_certificate(tx)))?;

        let message = NewViewMessage { high_qc, new_height };

        info!(target: LOG_TARGET, "🔥 Send NEWVIEW to {}", next_leader);
        self.tx_leader
            .send((next_leader.clone(), HotstuffMessage::NewView(message)))
            .await
            .map_err(|_| HotStuffError::InternalChannelClosed {
                context: "tx_leader in OnNextSyncViewHandler::send_to_leader",
            })
    }
}
