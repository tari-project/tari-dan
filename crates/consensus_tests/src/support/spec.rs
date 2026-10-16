//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_consensus::traits::{ConsensusSpec, hooks::NoopHooks};

use super::TestBlockTransactionProcessor;
use crate::support::{
    RoundRobinLeaderStrategy,
    address::TestAddress,
    epoch_manager::TestEpochManager,
    messaging_impls::{TestInboundMessaging, TestOutboundMessaging},
    signing_service::TestVoteSignatureService,
    sync::TestSyncManager,
};

pub type TestStore = tari_state_store_rocksdb::RocksDbStateStore<TestAddress>;

#[derive(Clone)]
pub struct TestConsensusSpec;

impl ConsensusSpec for TestConsensusSpec {
    type Addr = TestAddress;
    type EpochManager = TestEpochManager;
    type Hooks = NoopHooks;
    type InboundMessaging = TestInboundMessaging;
    type LeaderStrategy = RoundRobinLeaderStrategy;
    type OutboundMessaging = TestOutboundMessaging;
    type SignerService = TestVoteSignatureService;
    type StateStore = TestStore;
    type SyncManager = TestSyncManager;
    type TransactionExecutor = TestBlockTransactionProcessor;
    type TransactionValidator = TestBlockTransactionProcessor;
}
