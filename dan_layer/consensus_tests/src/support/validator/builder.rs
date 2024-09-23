//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::time::Duration;

use tari_common::configuration::Network;
use tari_common_types::types::{PrivateKey, PublicKey};
use tari_consensus::{
    hotstuff::{ConsensusCurrentState, ConsensusWorker, ConsensusWorkerContext, HotstuffConfig, HotstuffWorker},
    traits::hooks::NoopHooks,
};
use tari_crypto::keys::PublicKey as _;
use tari_dan_common_types::{ShardGroup, SubstateAddress};
use tari_dan_storage::consensus_models::TransactionPool;
use tari_shutdown::ShutdownSignal;
use tari_state_store_sqlite::SqliteStateStore;
use tokio::sync::{broadcast, mpsc, watch};

use crate::support::{
    address::TestAddress,
    epoch_manager::TestEpochManager,
    executions_store::TestExecutionSpecStore,
    messaging_impls::{TestInboundMessaging, TestOutboundMessaging},
    signing_service::TestVoteSignatureService,
    sync::AlwaysSyncedSyncManager,
    RoundRobinLeaderStrategy,
    TestBlockTransactionProcessor,
    TestConsensusSpec,
    Validator,
    ValidatorChannels,
    TEST_NUM_PRESHARDS,
};

pub struct ValidatorBuilder {
    pub address: TestAddress,
    pub secret_key: PrivateKey,
    pub public_key: PublicKey,
    pub shard_address: SubstateAddress,
    pub shard_group: ShardGroup,
    pub sql_url: String,
    pub leader_strategy: RoundRobinLeaderStrategy,
    pub num_committees: u32,
    pub epoch_manager: Option<TestEpochManager>,
    pub transaction_executions: TestExecutionSpecStore,
}

impl ValidatorBuilder {
    pub fn new() -> Self {
        Self {
            address: TestAddress::new("default"),
            secret_key: PrivateKey::default(),
            public_key: PublicKey::default(),
            shard_address: SubstateAddress::zero(),
            num_committees: 0,
            shard_group: ShardGroup::all_shards(TEST_NUM_PRESHARDS),
            sql_url: ":memory".to_string(),
            leader_strategy: RoundRobinLeaderStrategy::new(),
            epoch_manager: None,
            transaction_executions: TestExecutionSpecStore::new(),
        }
    }

    pub fn with_address_and_secret_key(&mut self, address: TestAddress, secret_key: PrivateKey) -> &mut Self {
        self.address = address;
        self.public_key = PublicKey::from_secret_key(&secret_key);
        self.secret_key = secret_key;
        self
    }

    pub fn with_shard_group(&mut self, shard_group: ShardGroup) -> &mut Self {
        self.shard_group = shard_group;
        self
    }

    pub fn with_shard(&mut self, shard: SubstateAddress) -> &mut Self {
        self.shard_address = shard;
        self
    }

    pub fn with_epoch_manager(&mut self, epoch_manager: TestEpochManager) -> &mut Self {
        self.epoch_manager = Some(epoch_manager);
        self
    }

    pub fn with_sql_url<T: Into<String>>(&mut self, sql_url: T) -> &mut Self {
        self.sql_url = sql_url.into();
        self
    }

    pub fn with_leader_strategy(&mut self, leader_strategy: RoundRobinLeaderStrategy) -> &mut Self {
        self.leader_strategy = leader_strategy;
        self
    }

    pub fn with_num_committees(&mut self, num_committees: u32) -> &mut Self {
        self.num_committees = num_committees;
        self
    }

    pub fn spawn(&self, shutdown_signal: ShutdownSignal) -> (ValidatorChannels, Validator) {
        log::info!(
            "Spawning validator with address {} and public key {}",
            self.address,
            self.public_key
        );

        let (tx_broadcast, rx_broadcast) = mpsc::channel(100);
        let (tx_new_transactions, rx_new_transactions) = mpsc::channel(100);
        let (tx_hs_message, rx_hs_message) = mpsc::channel(100);
        let (tx_leader, rx_leader) = mpsc::channel(100);

        let epoch_manager = self.epoch_manager.as_ref().unwrap().clone_for(
            self.address.clone(),
            self.public_key.clone(),
            self.shard_address,
        );

        let (outbound_messaging, rx_loopback) =
            TestOutboundMessaging::create(epoch_manager.clone(), tx_leader, tx_broadcast);
        let inbound_messaging = TestInboundMessaging::new(self.address.clone(), rx_hs_message, rx_loopback);

        let store = SqliteStateStore::connect(&self.sql_url).unwrap();
        let signing_service = TestVoteSignatureService::new(self.address.clone());
        let transaction_pool = TransactionPool::new();
        let (tx_events, _) = broadcast::channel(100);

        let transaction_executor = TestBlockTransactionProcessor::new(self.transaction_executions.clone());

        let worker = HotstuffWorker::<TestConsensusSpec>::new(
            HotstuffConfig {
                num_preshards: TEST_NUM_PRESHARDS,
                max_base_layer_blocks_ahead: 5,
                max_base_layer_blocks_behind: 5,
                network: Network::LocalNet,
                pacemaker_max_base_time: Duration::from_secs(10),
                sidechain_id: None,
            },
            self.address.clone(),
            inbound_messaging,
            outbound_messaging,
            rx_new_transactions,
            store.clone(),
            epoch_manager.clone(),
            self.leader_strategy,
            signing_service,
            transaction_pool,
            transaction_executor,
            tx_events.clone(),
            NoopHooks,
            shutdown_signal.clone(),
        );

        let (tx_current_state, rx_current_state) = watch::channel(ConsensusCurrentState::default());
        let context = ConsensusWorkerContext {
            epoch_manager: epoch_manager.clone(),
            hotstuff: worker,
            state_sync: AlwaysSyncedSyncManager,
            tx_current_state: tx_current_state.clone(),
        };

        let mut worker = ConsensusWorker::new(shutdown_signal);
        let handle = tokio::spawn(async move { worker.run(context).await });

        let channels = ValidatorChannels {
            address: self.address.clone(),
            shard_group: self.shard_group,
            num_committees: self.num_committees,
            state_store: store.clone(),
            tx_new_transactions,
            tx_hs_message,
            rx_broadcast,
            rx_leader,
        };

        let validator = Validator {
            address: self.address.clone(),
            shard_address: self.shard_address,
            shard_group: self.shard_group,
            num_committees: self.num_committees,
            transaction_executions: self.transaction_executions.clone(),
            state_store: store,
            epoch_manager,
            events: tx_events.subscribe(),
            current_state_machine_state: rx_current_state,
            handle,
        };
        (channels, validator)
    }
}
