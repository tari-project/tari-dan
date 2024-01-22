//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use sqlite_message_logger::SqliteMessageLogger;
use tari_consensus::traits::ConsensusSpec;
use tari_dan_common_types::PeerAddress;
use tari_epoch_manager::base_layer::EpochManagerHandle;
use tari_rpc_state_sync::RpcStateSyncManager;
use tari_state_store_sqlite::SqliteStateStore;

use crate::{
    consensus::{
        leader_selection::RoundRobinLeaderStrategy,
        signature_service::TariSignatureService,
        state_manager::TariStateManager,
    },
    p2p::services::messaging::{ConsensusInboundMessaging, ConsensusOutboundMessaging},
};

#[derive(Clone)]
pub struct TariConsensusSpec;

impl ConsensusSpec for TariConsensusSpec {
    type Addr = PeerAddress;
    type EpochManager = EpochManagerHandle<Self::Addr>;
    type InboundMessaging = ConsensusInboundMessaging<SqliteMessageLogger>;
    type LeaderStrategy = RoundRobinLeaderStrategy;
    type OutboundMessaging = ConsensusOutboundMessaging<SqliteMessageLogger>;
    type SignatureService = TariSignatureService;
    type StateManager = TariStateManager;
    type StateStore = SqliteStateStore<Self::Addr>;
    type SyncManager = RpcStateSyncManager<Self>;
}
