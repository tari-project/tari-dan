//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use log::*;
use tari_common_types::types::FixedHash;
use tari_epoch_manager::epoch_event_oracle::{EpochEvent, EpochEventOracle};
use tari_ootle_common_types::Epoch;
use tokio::sync::mpsc;

use crate::{
    base_layer::{BaseLayerBlockHeaderStore, BaseLayerOracle},
    configured::{ConfiguredEpochOracle, EpochTickerData},
    hybrid::mpsc_ticker::MpscEpochTicker,
    store::EpochOracleStore,
};

const LOG_TARGET: &str = "tari::ootle::epoch_oracles::hybrid";

pub struct HybridEpochOracle<TStore> {
    configured: ConfiguredEpochOracle<TStore, MpscEpochTicker>,
    base_layer: BaseLayerOracle<TStore>,
    trigger: mpsc::UnboundedSender<EpochTickerData>,
    has_initial_sync_completed: bool,
}

impl<TStore: EpochOracleStore + BaseLayerBlockHeaderStore + Send + 'static> HybridEpochOracle<TStore> {
    pub fn new(
        configured: ConfiguredEpochOracle<TStore, MpscEpochTicker>,
        base_layer: BaseLayerOracle<TStore>,
        trigger: mpsc::UnboundedSender<EpochTickerData>,
    ) -> Self {
        Self {
            configured,
            base_layer,
            trigger,
            has_initial_sync_completed: false,
        }
    }
}

impl<TStore: EpochOracleStore + BaseLayerBlockHeaderStore + Send + 'static> EpochEventOracle
    for HybridEpochOracle<TStore>
{
    async fn next_epoch_event(&mut self) -> Option<EpochEvent> {
        loop {
            tokio::select! {
                biased;

                event = self.configured.next_epoch_event() => {
                    let event = event?;
                    debug!(target: LOG_TARGET, "📝 Configured epoch event: {}", event);
                        return Some(event);
                },
                event = self.base_layer.next_epoch_event() => {
                    let event = event?;
                    debug!(target: LOG_TARGET, "⛓️ Base layer epoch event: {}", event);
                    match event {
                        // Trigger the watch sender for epoch changes
                        EpochEvent::EpochChanged { epoch, epoch_hash } => {
                            let _ignore = self.trigger.send(EpochTickerData {
                                epoch,
                                epoch_hash,
                                done_for_now: self.has_initial_sync_completed,
                            });
                        },
                        EpochEvent::DoneForNow {epoch, epoch_hash} if !self.has_initial_sync_completed => {
                            let _ignore = self.trigger.send(EpochTickerData{
                                epoch,
                                epoch_hash,
                                done_for_now: true,
                            });
                            self.has_initial_sync_completed = true;
                        },
                        // Ignore other events that are not epoch changes
                        _ => {},
                    }
                },
            }
        }
    }

    fn is_within_epoch_end_spread(&self, current_epoch: Epoch) -> bool {
        // Epoch timing in hybrid mode is driven by the base-layer scanner; defer to it.
        self.base_layer.is_within_epoch_end_spread(current_epoch)
    }

    fn observed_epoch_boundary_hash(&self, epoch: Epoch) -> anyhow::Result<Option<FixedHash>> {
        // Epoch boundaries in hybrid mode are scanned by the base-layer oracle; defer to it.
        self.base_layer.observed_epoch_boundary_hash(epoch)
    }
}
