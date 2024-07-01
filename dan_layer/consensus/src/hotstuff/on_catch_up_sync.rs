//    Copyright 2024 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use log::{info, warn};
use tari_dan_common_types::Epoch;
use tari_dan_storage::{consensus_models::HighQc, StateStore};
use tari_epoch_manager::EpochManagerReader;

use crate::{
    hotstuff::{pacemaker_handle::PaceMakerHandle, HotStuffError},
    messages::{HotstuffMessage, SyncRequestMessage},
    traits::{ConsensusSpec, OutboundMessaging},
};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::on_catch_up_sync";

pub struct OnCatchUpSync<TConsensusSpec: ConsensusSpec> {
    store: TConsensusSpec::StateStore,
    pacemaker: PaceMakerHandle,
    outbound_messaging: TConsensusSpec::OutboundMessaging,
    epoch_manager: TConsensusSpec::EpochManager,
}

impl<TConsensusSpec: ConsensusSpec> OnCatchUpSync<TConsensusSpec> {
    pub fn new(
        store: TConsensusSpec::StateStore,
        pacemaker: PaceMakerHandle,
        outbound_messaging: TConsensusSpec::OutboundMessaging,
        epoch_manager: TConsensusSpec::EpochManager,
    ) -> Self {
        Self {
            store,
            pacemaker,
            outbound_messaging,
            epoch_manager,
        }
    }

    pub async fn request_sync(&mut self, epoch: Epoch, from: &TConsensusSpec::Addr) -> Result<(), HotStuffError> {
        let high_qc = self.store.with_read_tx(|tx| HighQc::get(tx))?;
        info!(
            target: LOG_TARGET,
            "⏰ Catch up required from block {} from {} (current view: {})",
            high_qc,
            from,
            self.pacemaker.current_view()
        );
        // Reset leader timeout since we're behind. TODO: This is hacky.
        self.pacemaker
            .reset_view(epoch, high_qc.block_height(), high_qc.block_height)
            .await?;

        let current_epoch = self.epoch_manager.current_epoch().await?;
        // Request a catch-up
        if self
            .outbound_messaging
            .send(
                from.clone(),
                HotstuffMessage::CatchUpSyncRequest(SyncRequestMessage {
                    epoch: current_epoch,
                    high_qc,
                }),
            )
            .await
            .is_err()
        {
            warn!(target: LOG_TARGET, "Leader channel closed while sending SyncRequest");
            return Ok(());
        }

        Ok(())
    }
}
