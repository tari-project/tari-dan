//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use tari_comms::types::CommsPublicKey;
use tari_comms_rpc_state_sync::CommsRpcStateSyncManager;
use tari_consensus::traits::ConsensusSpec;
use tari_epoch_manager::base_layer::EpochManagerHandle;
use tari_state_store_sqlite::SqliteStateStore;

use crate::consensus::{
    leader_selection::RoundRobinLeaderStrategy,
    signature_service::TariSignatureService,
    state_manager::TariStateManager,
};

pub struct TariConsensusSpec;

impl ConsensusSpec for TariConsensusSpec {
    type Addr = CommsPublicKey;
    type EpochManager = EpochManagerHandle;
    type LeaderStrategy = RoundRobinLeaderStrategy;
    type StateManager = TariStateManager;
    type StateStore = SqliteStateStore<Self::Addr>;
    type SyncManager = CommsRpcStateSyncManager<Self::EpochManager, Self::StateStore>;
    type VoteSignatureService = TariSignatureService;
}
