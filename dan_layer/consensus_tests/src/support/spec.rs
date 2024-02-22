//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_consensus::traits::{hooks::NoopHooks, ConsensusSpec};
use tari_state_store_sqlite::SqliteStateStore;

use super::TestTransactionProcessor;
use crate::support::{
    address::TestAddress,
    epoch_manager::TestEpochManager,
    messaging_impls::{TestInboundMessaging, TestOutboundMessaging},
    signing_service::TestVoteSignatureService,
    sync::AlwaysSyncedSyncManager,
    NoopStateManager,
    RoundRobinLeaderStrategy,
};

#[derive(Clone)]
pub struct TestConsensusSpec;

impl ConsensusSpec for TestConsensusSpec {
    type Addr = TestAddress;
    type EpochManager = TestEpochManager;
    type Hooks = NoopHooks;
    type InboundMessaging = TestInboundMessaging;
    type LeaderStrategy = RoundRobinLeaderStrategy;
    type OutboundMessaging = TestOutboundMessaging;
    type SignatureService = TestVoteSignatureService;
    type StateManager = NoopStateManager;
    type StateStore = SqliteStateStore<Self::Addr>;
    type SyncManager = AlwaysSyncedSyncManager;
    type TransactionExecutor = TestTransactionProcessor;
}
