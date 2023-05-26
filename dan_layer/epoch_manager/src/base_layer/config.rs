//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

#[derive(Debug, Clone)]
pub struct EpochManagerConfig {
    pub base_layer_confirmations: u64,
    pub committee_size: u64,
}
