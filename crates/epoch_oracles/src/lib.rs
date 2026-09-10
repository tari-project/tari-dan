//    Copyright 2025 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use tari_common_types::types::FixedHash;
use tari_epoch_manager::epoch_event_oracle::{EpochEvent, EpochEventOracle};
use tari_ootle_common_types::Epoch;

use crate::{configured::RealTimeEpochTicker, store::EpochOracleStore};

#[cfg(feature = "base_layer")]
pub mod base_layer;
pub mod configured;
#[cfg(feature = "base_layer")]
pub mod hybrid;
pub mod store;

pub enum EpochOracle<TStore> {
    #[cfg(feature = "base_layer")]
    BaseLayer(base_layer::BaseLayerOracle<TStore>),
    Configured(configured::ConfiguredEpochOracle<TStore, RealTimeEpochTicker>),
    #[cfg(feature = "base_layer")]
    Hybrid(hybrid::HybridEpochOracle<TStore>),
}

#[cfg(feature = "base_layer")]
impl<TStore> EpochEventOracle for EpochOracle<TStore>
where
    TStore: EpochOracleStore + Send + 'static,
    TStore: base_layer::BaseLayerBlockHeaderStore,
{
    async fn next_epoch_event(&mut self) -> Option<EpochEvent> {
        match self {
            EpochOracle::BaseLayer(base_layer) => base_layer.next_epoch_event().await,
            EpochOracle::Configured(configured) => configured.next_epoch_event().await,
            EpochOracle::Hybrid(hybrid) => hybrid.next_epoch_event().await,
        }
    }

    fn observed_epoch_boundary_hash(&self, epoch: Epoch) -> anyhow::Result<Option<FixedHash>> {
        match self {
            EpochOracle::BaseLayer(base_layer) => base_layer.observed_epoch_boundary_hash(epoch),
            EpochOracle::Configured(configured) => configured.observed_epoch_boundary_hash(epoch),
            EpochOracle::Hybrid(hybrid) => hybrid.observed_epoch_boundary_hash(epoch),
        }
    }
}

#[cfg(not(feature = "base_layer"))]
impl<TStore> EpochEventOracle for EpochOracle<TStore>
where TStore: EpochOracleStore + Send + 'static
{
    async fn next_epoch_event(&mut self) -> Option<EpochEvent> {
        match self {
            EpochOracle::Configured(configured) => configured.next_epoch_event().await,
        }
    }

    fn observed_epoch_boundary_hash(&self, epoch: Epoch) -> anyhow::Result<Option<FixedHash>> {
        match self {
            EpochOracle::Configured(configured) => configured.observed_epoch_boundary_hash(epoch),
        }
    }
}
