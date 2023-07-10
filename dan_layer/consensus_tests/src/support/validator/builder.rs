//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_consensus::hotstuff::HotstuffWorker;
use tari_dan_common_types::{Epoch, ShardId};
use tari_dan_storage::consensus_models::TransactionPool;
use tari_epoch_manager::EpochManagerEvent;
use tari_shutdown::Shutdown;
use tari_state_store_sqlite::SqliteStateStore;
use tokio::sync::{broadcast, mpsc};

use crate::support::{
    address::TestAddress,
    epoch_manager::TestEpochManager,
    signing_service::TestVoteSignatureService,
    NoopStateManager,
    SelectedIndexLeaderStrategy,
    TestConsensusSpec,
    Validator,
    ValidatorChannels,
};

pub struct ValidatorBuilder {
    pub address: TestAddress,
    pub shard: ShardId,
    pub sql_url: String,
    pub leader_strategy: SelectedIndexLeaderStrategy,
    pub epoch_manager: TestEpochManager,
}

impl ValidatorBuilder {
    pub fn new() -> Self {
        Self {
            address: TestAddress("default"),
            shard: ShardId::zero(),
            sql_url: ":memory".to_string(),
            leader_strategy: SelectedIndexLeaderStrategy::new(0),
            epoch_manager: TestEpochManager::new(),
        }
    }

    pub fn with_address(&mut self, address: TestAddress) -> &mut Self {
        self.address = address;
        self
    }

    pub fn with_shard(&mut self, shard: ShardId) -> &mut Self {
        self.shard = shard;
        self
    }

    pub fn with_epoch_manager(&mut self, epoch_manager: TestEpochManager) -> &mut Self {
        self.epoch_manager = epoch_manager;
        self
    }

    pub fn with_sql_url<T: Into<String>>(&mut self, sql_url: T) -> &mut Self {
        self.sql_url = sql_url.into();
        self
    }

    pub fn with_leader_strategy(&mut self, leader_strategy: SelectedIndexLeaderStrategy) -> &mut Self {
        self.leader_strategy = leader_strategy;
        self
    }

    pub fn spawn(&self) -> (ValidatorChannels, Validator) {
        let (tx_broadcast, rx_broadcast) = mpsc::channel(10);
        let (tx_new_transactions, rx_new_transactions) = mpsc::channel(100);
        let (tx_hs_message, rx_hs_message) = mpsc::channel(10);
        let (tx_leader, rx_leader) = mpsc::channel(10);

        let store = SqliteStateStore::connect(&self.sql_url).unwrap();
        let signing_service = TestVoteSignatureService::new();
        let shutdown = Shutdown::new();
        let shutdown_signal = shutdown.to_signal();
        let transaction_pool = TransactionPool::new();
        let noop_state_manager = NoopStateManager::new();
        let (tx_events, _) = broadcast::channel(100);
        let (tx_epoch_events, rx_epoch_events) = broadcast::channel(1);

        let worker = HotstuffWorker::<TestConsensusSpec>::new(
            self.address,
            rx_new_transactions,
            rx_hs_message,
            store.clone(),
            rx_epoch_events,
            self.epoch_manager.clone_for(self.address, self.shard),
            self.leader_strategy.clone(),
            signing_service,
            noop_state_manager,
            transaction_pool,
            tx_broadcast,
            tx_leader,
            tx_events.clone(),
            tx_new_transactions.clone(),
            shutdown_signal,
        );
        let handle = tokio::spawn(async move {
            worker.run().await.unwrap();
        });

        let channels = ValidatorChannels {
            address: self.address,
            tx_new_transactions,
            tx_hs_message,
            rx_broadcast,
            rx_leader,
        };

        // Fire off initial epoch change event
        tx_epoch_events.send(EpochManagerEvent::EpochChanged(Epoch(0))).unwrap();

        let validator = Validator {
            address: self.address,
            shard: self.shard,
            state_store: store,
            epoch_manager: self.epoch_manager.clone_for(self.address, self.shard),
            shutdown,
            tx_epoch_events,
            leader_strategy: self.leader_strategy.clone(),
            events: tx_events.subscribe(),
            handle,
        };
        (channels, validator)
    }
}
