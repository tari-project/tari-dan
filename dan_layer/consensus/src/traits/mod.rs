//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

mod leader_strategy;
mod signing_service;
mod state_manager;
mod sync;

pub use leader_strategy::*;
use serde::Serialize;
pub use state_manager::*;
pub use sync::*;
use tari_dan_common_types::NodeAddressable;
use tari_dan_storage::StateStore;
use tari_epoch_manager::EpochManagerReader;

pub use crate::traits::signing_service::*;

pub trait ConsensusSpec: Send + Sync + 'static {
    type Addr: NodeAddressable + Serialize;

    type StateStore: StateStore<Addr = Self::Addr> + Send + Sync + 'static;
    type EpochManager: EpochManagerReader<Addr = Self::Addr> + Send + Sync + 'static;
    type LeaderStrategy: LeaderStrategy<Self::Addr> + Send + Sync + 'static;
    type VoteSignatureService: VoteSignatureService<Self::Addr> + Send + Sync + 'static;
    type StateManager: StateManager<Self::StateStore> + Send + Sync + 'static;
    type SyncManager: SyncManager + Send + Sync + 'static;
}
