//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_consensus::traits::ConsensusSpec;
use tari_state_store_sqlite::SqliteStateStore;

use crate::support::{
    address::TestAddress,
    epoch_manager::TestEpochManager,
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
    type LeaderStrategy = RoundRobinLeaderStrategy;
    type StateManager = NoopStateManager;
    type StateStore = SqliteStateStore<Self::Addr>;
    type SyncManager = AlwaysSyncedSyncManager;
    type VoteSignatureService = TestVoteSignatureService;
}
