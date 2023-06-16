//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_consensus::hotstuff::HotstuffWorker;
use tari_shutdown::Shutdown;
use tari_state_store_sqlite::SqliteStateStore;
use tokio::sync::mpsc;

use crate::support::{
    address::TestAddress,
    epoch_manager::TestEpochManager,
    signing_service::TestVoteSigningService,
    SelectedIndexLeaderStrategy,
    TestConsensusSpec,
    Validator,
};

pub struct ValidatorBuilder {
    pub address: TestAddress,
    pub sql_url: String,
    pub leader_strategy: SelectedIndexLeaderStrategy,
    pub epoch_manager: TestEpochManager,
}

impl ValidatorBuilder {
    pub fn new() -> Self {
        Self {
            address: TestAddress("default"),
            sql_url: ":memory".to_string(),
            leader_strategy: SelectedIndexLeaderStrategy::new(0),
            epoch_manager: TestEpochManager::new(),
        }
    }

    pub fn with_address(&mut self, address: TestAddress) -> &mut Self {
        self.address = address;
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

    pub fn spawn(&self) -> Validator {
        let (tx_broadcast, rx_broadcast) = mpsc::channel(1);
        let (tx_new_transactions, rx_new_transactions) = mpsc::channel(100);
        let (tx_hs_message, rx_hs_message) = mpsc::channel(1);
        let (tx_leader, rx_leader) = mpsc::channel(1);

        let store = SqliteStateStore::connect(&self.sql_url).unwrap();
        let signing_service = TestVoteSigningService::new();
        let shutdown = Shutdown::new();
        let shutdown_signal = shutdown.to_signal();

        let worker = HotstuffWorker::<TestConsensusSpec>::new(
            self.address,
            rx_new_transactions,
            rx_hs_message,
            store.clone(),
            self.epoch_manager.copy_for(self.address),
            self.leader_strategy.clone(),
            signing_service,
            tx_broadcast,
            tx_leader,
            shutdown_signal,
        );
        let handle = tokio::spawn(async move {
            worker.run().await.unwrap();
        });

        Validator {
            address: self.address,
            tx_new_transactions,
            tx_hs_message,
            rx_broadcast,
            rx_leader,
            state_store: store,
            epoch_manager: self.epoch_manager.copy_for(self.address),
            shutdown,
            leader_strategy: self.leader_strategy.clone(),
            handle,
        }
    }
}
