//  Copyright 2021. The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::time::Duration;

use log::*;
use tari_comms::{connection_manager::LivenessStatus, connectivity::ConnectivityEvent, peer_manager::NodeId};
use tari_consensus::hotstuff::HotstuffEvent;
use tari_dan_storage::{consensus_models::Block, StateStore};
use tari_epoch_manager::{EpochManagerError, EpochManagerEvent, EpochManagerReader};
use tari_shutdown::ShutdownSignal;
use tokio::{task, time, time::MissedTickBehavior};

use crate::{
    p2p::services::{committee_state_sync::CommitteeStateSync, networking::NetworkingService},
    Services,
};

const LOG_TARGET: &str = "tari::validator_node::dan_node";

pub struct DanNode {
    services: Services,
}

impl DanNode {
    pub fn new(services: Services) -> Self {
        Self { services }
    }

    pub async fn start(mut self, mut shutdown: ShutdownSignal) -> Result<(), anyhow::Error> {
        let mut hotstuff_events = self.services.hotstuff_events.subscribe();

        let mut connectivity_events = self.services.comms.connectivity().get_event_subscription();

        if let Err(err) = self.dial_local_shard_peers().await {
            error!(target: LOG_TARGET, "Failed to dial local shard peers: {}", err);
        }

        let status = self.services.comms.connectivity().get_connectivity_status().await?;
        if status.is_online() {
            self.services.networking.announce().await?;
        }

        let mut current_inbound_status = self.services.comms.liveness_status();
        let mut tick = time::interval(Duration::from_secs(10));
        tick.set_missed_tick_behavior(MissedTickBehavior::Skip);

        let mut epoch_manager_events = self.services.epoch_manager.subscribe().await?;

        loop {
            tokio::select! {
                // Wait until killed
                _ = shutdown.wait() => {
                     break;
                },

                Ok(event) = connectivity_events.recv() => {
                    if let ConnectivityEvent::ConnectivityStateOnline(_) = event {
                        // We're back online, announce
                        if let Err(err) = self.services.networking.announce().await {
                            error!(target: LOG_TARGET, "Failed to announce: {}", err);
                        }
                    }
                },

                Ok(event) = hotstuff_events.recv() => if let Err(err) = self.handle_hotstuff_event(event).await{
                    error!(target: LOG_TARGET, "Error handling hotstuff event: {}", err);
                },

                Ok(event) = epoch_manager_events.recv() => {
                    self.handle_epoch_manager_event(event).await?;
                }

                Err(err) = self.services.on_any_exit() => {
                    error!(target: LOG_TARGET, "Error in service: {}", err);
                    return Err(err);
                }

                _ = tick.tick() => {
                    let status = self.services.comms.liveness_status() ;
                    match status {
                        LivenessStatus::Disabled | LivenessStatus::Checking => {},
                        LivenessStatus::Unreachable => { warn!(target: LOG_TARGET, "🔌 Node is unreachable"); }
                        LivenessStatus::Live(t) => {
                            if !matches!(current_inbound_status, LivenessStatus::Live(_)) {
                                info!(target: LOG_TARGET, "⚡️ Node is reachable (ping {:.2?})", t);
                            }
                        }
                    }
                    current_inbound_status = status;
                }
            }
        }

        Ok(())
    }

    async fn handle_hotstuff_event(&self, event: HotstuffEvent) -> Result<(), anyhow::Error> {
        let HotstuffEvent::BlockCommitted { block_id, .. } = event else {
            return Ok(());
        };

        let committed_transactions = self.services.state_store.with_read_tx(|tx| {
            let block = Block::get(tx, &block_id)?;
            info!(target: LOG_TARGET, "🏁 Block {} committed", block_id);
            Ok::<_, anyhow::Error>(
                block
                    .commands()
                    .iter()
                    .filter_map(|cmd| cmd.accept())
                    .map(|t| t.id)
                    .collect::<Vec<_>>(),
            )
        })?;

        info!(target: LOG_TARGET, "🏁 Removing {} finalized transaction(s) from mempool", committed_transactions.len());
        for tx_id in committed_transactions {
            if let Err(err) = self.services.mempool.remove_transaction(tx_id).await {
                error!(target: LOG_TARGET, "Failed to remove transaction from mempool: {}", err);
            }
        }

        Ok(())
    }

    async fn handle_epoch_manager_event(&self, event: EpochManagerEvent) -> Result<(), anyhow::Error> {
        match event {
            EpochManagerEvent::EpochChanged(epoch) => {
                info!(target: LOG_TARGET, "📅 Epoch changed to {}", epoch);
                let sync_service = CommitteeStateSync::new(
                    self.services.epoch_manager.clone(),
                    self.services.validator_node_client_factory.clone(),
                    self.services.state_store.clone(),
                    self.services.global_db.clone(),
                    self.services.comms.node_identity().public_key().clone(),
                );

                // EpochChanged should only happen once per epoch and the event is not emitted during initial sync. So
                // spawning state sync for each event should be ok.
                task::spawn(async move {
                    if let Err(e) = sync_service.sync_state(epoch).await {
                        error!(
                            target: LOG_TARGET,
                            "Failed to sync peers state for epoch {}: {}", epoch, e
                        );
                    }
                });
            },
            EpochManagerEvent::ThisValidatorIsRegistered { .. } => {},
        }
        Ok(())
    }

    async fn dial_local_shard_peers(&mut self) -> Result<(), anyhow::Error> {
        let epoch = self.services.epoch_manager.current_epoch().await?;
        let res = self
            .services
            .epoch_manager
            .get_validator_node(epoch, self.services.comms.node_identity().public_key())
            .await;

        let shard_id = match res {
            Ok(vn) => vn.shard_key,
            Err(EpochManagerError::ValidatorNodeNotRegistered { address }) => {
                info!(target: LOG_TARGET, "Validator node {address} registered for this epoch");
                return Ok(());
            },
            Err(EpochManagerError::BaseLayerConsensusConstantsNotSet) => {
                info!(target: LOG_TARGET, "Epoch manager has not synced with base layer yet");
                return Ok(());
            },
            Err(err) => {
                return Err(err.into());
            },
        };

        let local_shard_peers = self.services.epoch_manager.get_committee(epoch, shard_id).await?;
        info!(
            target: LOG_TARGET,
            "Dialing {} local shard peers",
            local_shard_peers.members.len()
        );

        self.services
            .comms
            .connectivity()
            .request_many_dials(
                local_shard_peers
                    .members
                    .into_iter()
                    .filter(|pk| pk != self.services.comms.node_identity().public_key())
                    .map(|pk| NodeId::from_public_key(&pk)),
            )
            .await?;
        Ok(())
    }
}
