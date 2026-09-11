//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_bor::{Deserialize, Serialize};
use tari_engine_types::UtxoOutput;
use tari_ootle_common_types::{StateVersion, shard::Shard};
use tari_template_lib_types::UtxoAddress;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UtxoUpdateRecord {
    Unspent(Box<UtxoUnspent>),
    Spent(UtxoSpent),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UtxoUnspent {
    pub address: UtxoAddress,
    pub version: u64,
    pub shard: Shard,
    pub state_version: StateVersion,
    pub utxo_output: UtxoOutput,
    pub is_frozen: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UtxoSpent {
    pub address: UtxoAddress,
    pub version: u64,
    pub shard: Shard,
    pub state_version: StateVersion,
}
