//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

mod base_layer_epoch_manager;
mod config;
mod epoch_manager_service;
mod error;
mod handle;
mod initializer;
mod types;

pub use base_layer_epoch_manager::BaseLayerEpochManager;
pub use config::EpochManagerConfig;
pub use epoch_manager_service::EpochManagerService;
pub use error::EpochManagerError;
pub use handle::EpochManagerHandle;
pub use initializer::spawn_service;
pub use types::*;
