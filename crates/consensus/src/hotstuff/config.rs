//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::time::Duration;

use tari_ootle_transaction::Network;
use tari_template_lib_types::crypto::RistrettoPublicKeyBytes;

use crate::consensus_constants::ConsensusConstants;

#[derive(Debug, Clone)]
pub struct HotstuffConfig {
    pub network: Network,
    pub sidechain_id: Option<RistrettoPublicKeyBytes>,
    pub consensus_constants: ConsensusConstants,
    /// The number of state versions the state store keeps the tree nodes of. A substate's down proof is built from a
    /// version within it.
    pub state_history_length: u64,
    pub state_tree_cleanup_interval: Duration,
    pub epoch_gc_interval: Duration,
    pub epoch_end_grace_period: Duration,
    pub catch_up_request_timeout: Duration,
}
