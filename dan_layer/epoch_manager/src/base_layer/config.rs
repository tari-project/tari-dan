//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_common_types::types::PublicKey;

#[derive(Debug, Clone)]
pub struct EpochManagerConfig {
    pub base_layer_confirmations: u64,
    pub committee_size: u32,
    pub validator_node_sidechain_id : Option<PublicKey>
}
