//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::time::Duration;

use tari_common::configuration::Network;
use tari_crypto::ristretto::RistrettoPublicKey;
use tari_dan_common_types::NumPreshards;

#[derive(Debug, Clone)]
pub struct HotstuffConfig {
    pub network: Network,
    pub max_base_layer_blocks_ahead: u64,
    pub max_base_layer_blocks_behind: u64,
    pub num_preshards: NumPreshards,
    pub pacemaker_max_base_time: Duration,
    pub sidechain_id: Option<RistrettoPublicKey>,
}
