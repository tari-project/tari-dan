//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

// mod block_signer;
mod epoch_manager;
mod leader_strategy;

pub use epoch_manager::*;
pub use leader_strategy::*;
use tari_dan_storage::StateStore;

pub trait ConsensusSpec {
    type Error;
    type StateStore: StateStore;
    type EpochManager: EpochManager<Error = Self::Error>;
    type LeaderStrategy: LeaderStrategy;
}
