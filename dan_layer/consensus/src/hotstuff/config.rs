//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

#[derive(Debug, Clone)]
pub struct HotstuffConfig {
    pub max_base_layer_blocks_ahead: u64,
    pub max_base_layer_blocks_behind: u64,
}
