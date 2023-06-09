//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

// mod block_signer;
mod epoch_manager;
mod leader_strategy;

pub use epoch_manager::*;
pub use leader_strategy::*;
use tari_dan_common_types::NodeAddressable;
use tari_dan_storage::StateStore;

pub trait ConsensusSpec {
    type Addr: NodeAddressable;

    type StateStore: StateStore;
    type EpochManager: EpochManager<Addr = Self::Addr>;
    type LeaderStrategy: LeaderStrategy<Self::Addr>;
}
