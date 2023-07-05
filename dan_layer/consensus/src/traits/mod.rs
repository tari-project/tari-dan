//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

mod epoch_manager;
mod leader_strategy;
mod signing_service;
mod state_manager;

pub use epoch_manager::*;
pub use leader_strategy::*;
pub use state_manager::*;
use tari_dan_common_types::NodeAddressable;
use tari_dan_storage::StateStore;

pub use crate::traits::signing_service::*;

pub trait ConsensusSpec: Send + Sync + 'static {
    type Addr: NodeAddressable;

    type StateStore: StateStore + Send + Sync + 'static;
    type EpochManager: EpochManager<Addr = Self::Addr> + Send + Sync + 'static;
    type LeaderStrategy: LeaderStrategy<Self::Addr> + Send + Sync + 'static;
    type VoteSignatureService: VoteSignatureService + Send + Sync + 'static;
    type StateManager: StateManager<Self::StateStore> + Send + Sync + 'static;
}
