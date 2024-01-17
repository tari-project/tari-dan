//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_common_types::types::PublicKey;
use tari_consensus::hotstuff::{ConsensusWorker, ConsensusWorkerContext, HotstuffWorker};
use tari_dan_common_types::{shard_bucket::ShardBucket, ShardId};
use tari_dan_storage::consensus_models::{ForeignReceiveCounters, TransactionPool};
use tari_shutdown::ShutdownSignal;
use tari_state_store_sqlite::SqliteStateStore;
use tokio::sync::{broadcast, mpsc, watch};

use crate::support::{
    address::TestAddress,
    epoch_manager::TestEpochManager,
    messaging_impls::{TestInboundMessaging, TestOutboundMessaging},
    signing_service::TestVoteSignatureService,
    sync::AlwaysSyncedSyncManager,
    NoopStateManager,
    RoundRobinLeaderStrategy,
    TestConsensusSpec,
    Validator,
    ValidatorChannels,
};

pub struct ValidatorBuilder {
    pub address: TestAddress,
    pub public_key: PublicKey,
    pub shard: ShardId,
    pub bucket: ShardBucket,
    pub sql_url: String,
    pub leader_strategy: RoundRobinLeaderStrategy,
    pub epoch_manager: Option<TestEpochManager>,
}

impl ValidatorBuilder {
    pub fn new() -> Self {
        Self {
            address: TestAddress::new("default"),
            public_key: PublicKey::default(),
            shard: ShardId::zero(),
            bucket: ShardBucket::from(0),
            sql_url: ":memory".to_string(),
            leader_strategy: RoundRobinLeaderStrategy::new(),
            epoch_manager: None,
        }
    }

    pub fn with_address_and_public_key(&mut self, address: TestAddress, public_key: PublicKey) -> &mut Self {
        self.address = address;
        self.public_key = public_key;
        self
    }

    pub fn with_bucket(&mut self, bucket: ShardBucket) -> &mut Self {
        self.bucket = bucket;
        self
    }

    pub fn with_shard(&mut self, shard: ShardId) -> &mut Self {
        self.shard = shard;
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

    pub fn spawn(&self, shutdown_signal: ShutdownSignal) -> (ValidatorChannels, Validator) {
        let (tx_broadcast, rx_broadcast) = mpsc::channel(100);
        let (tx_new_transactions, rx_new_transactions) = mpsc::channel(100);
        let (tx_hs_message, rx_hs_message) = mpsc::channel(100);
        let (tx_leader, rx_leader) = mpsc::channel(100);
        let (tx_mempool, rx_mempool) = mpsc::unbounded_channel();

        let (outbound_messaging, rx_loopback) = TestOutboundMessaging::create(tx_leader, tx_broadcast);
        let inbound_messaging = TestInboundMessaging::new(self.address.clone(), rx_hs_message, rx_loopback);

        let store = SqliteStateStore::connect(&self.sql_url).unwrap();
        let signing_service = TestVoteSignatureService::new(self.public_key.clone(), self.address.clone());
        let transaction_pool = TransactionPool::new();
        let noop_state_manager = NoopStateManager::new();
        let (tx_events, _) = broadcast::channel(100);

        let epoch_manager =
            self.epoch_manager
                .as_ref()
                .unwrap()
                .clone_for(self.address.clone(), self.public_key.clone(), self.shard);
        let worker = HotstuffWorker::<TestConsensusSpec>::new(
            self.address.clone(),
            inbound_messaging,
            outbound_messaging,
            rx_new_transactions,
            store.clone(),
            epoch_manager.clone(),
            self.leader_strategy,
            signing_service,
            noop_state_manager.clone(),
            transaction_pool,
            tx_events.clone(),
            tx_mempool,
            ForeignReceiveCounters::default(),
            shutdown_signal.clone(),
        );

        let (tx_current_state, _) = watch::channel(Default::default());
        let context = ConsensusWorkerContext {
            epoch_manager: epoch_manager.clone(),
            hotstuff: worker,
            state_sync: AlwaysSyncedSyncManager,
            tx_current_state,
        };

        let mut worker = ConsensusWorker::new(shutdown_signal);
        let handle = tokio::spawn(async move { worker.run(context).await });

        let channels = ValidatorChannels {
            address: self.address.clone(),
            bucket: self.bucket,
            state_store: store.clone(),
            tx_new_transactions,
            tx_hs_message,
            rx_broadcast,
            rx_leader,
            rx_mempool,
        };

        let validator = Validator {
            address: self.address.clone(),
            shard: self.shard,
            state_store: store,
            epoch_manager,
            state_manager: noop_state_manager,
            leader_strategy: self.leader_strategy,
            events: tx_events.subscribe(),
            handle,
        };
        (channels, validator)
    }
}
