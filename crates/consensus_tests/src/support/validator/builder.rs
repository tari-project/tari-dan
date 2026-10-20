//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    ops::Deref,
    path::{Path, PathBuf},
    sync::Arc,
};

use serde::Serialize;
use tari_common_types::types::PrivateKey;
use tari_consensus::{
    hotstuff::{ConsensusCurrentState, ConsensusWorker, ConsensusWorkerContext, HotstuffConfig, HotstuffWorker},
    traits::hooks::NoopHooks,
};
use tari_crypto::{keys::PublicKey, ristretto::RistrettoPublicKey};
use tari_engine_types::substate::{SubstateId, SubstateValue};
use tari_ootle_common_types::{
    Epoch,
    NodeAddressable,
    ShardGroup,
    SubstateAddress,
    SubstateVersion,
    VersionedSubstateIdRef,
};
use tari_ootle_storage::{
    StateStore,
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
    consensus_models::{SubstateRecord, SubstateTransition, SubstateUpdateBatch, TransactionPool},
};
use tari_ootle_transaction::Network;
use tari_shutdown::ShutdownSignal;
use tari_state_store_rocksdb::DatabaseOptions;
use tari_template_lib_types::crypto::RistrettoPublicKeyBytes;
use tempfile::TempDir;
use tokio::sync::{broadcast, mpsc, watch};

use crate::support::{
    RoundRobinLeaderStrategy,
    TEST_NUM_PRESHARDS,
    TestBlockTransactionProcessor,
    TestConsensusSpec,
    TestStore,
    Validator,
    ValidatorChannels,
    address::TestAddress,
    epoch_manager::TestEpochManager,
    executions_store::TestExecutionSpecStore,
    messaging_impls::{NetworkSendObserver, SendObserver, TestInboundMessaging, TestOutboundMessaging},
    signing_service::TestVoteSignatureService,
    sync::TestSyncManager,
};

pub struct ValidatorBuilder {
    pub address: TestAddress,
    pub secret_key: PrivateKey,
    pub public_key: RistrettoPublicKey,
    pub shard_address: SubstateAddress,
    pub fee_claim_public_key: RistrettoPublicKeyBytes,
    pub shard_group: ShardGroup,
    pub rocks_tmp_path: TempDir,
    pub rocks_override_path: Option<PathBuf>,
    pub leader_strategy: RoundRobinLeaderStrategy,
    pub num_committees: u32,
    pub epoch_manager: Option<TestEpochManager>,
    pub transaction_executions: TestExecutionSpecStore,
    pub config: Option<HotstuffConfig>,
    pub send_observer: Option<NetworkSendObserver>,
}

impl ValidatorBuilder {
    pub fn new() -> Self {
        Self {
            address: TestAddress::new("default"),
            secret_key: PrivateKey::default(),
            public_key: RistrettoPublicKey::default(),
            fee_claim_public_key: RistrettoPublicKeyBytes::default(),
            shard_address: SubstateAddress::zero(),
            num_committees: 0,
            shard_group: ShardGroup::all_shards(TEST_NUM_PRESHARDS),
            rocks_override_path: None,
            rocks_tmp_path: tempfile::tempdir().unwrap(),
            leader_strategy: RoundRobinLeaderStrategy::new(),
            epoch_manager: None,
            transaction_executions: TestExecutionSpecStore::new(),
            config: None,
            send_observer: None,
        }
    }

    pub fn with_address_and_secret_key(&mut self, address: TestAddress, secret_key: PrivateKey) -> &mut Self {
        self.address = address;
        self.public_key = RistrettoPublicKey::from_secret_key(&secret_key);
        self.secret_key = secret_key;
        self
    }

    pub fn with_config(&mut self, config: HotstuffConfig) -> &mut Self {
        self.config = Some(config);
        self
    }

    pub fn with_shard_group(&mut self, shard_group: ShardGroup) -> &mut Self {
        self.shard_group = shard_group;
        self
    }

    pub fn with_fee_claim_public_key(&mut self, fee_claim_public_key: RistrettoPublicKeyBytes) -> &mut Self {
        self.fee_claim_public_key = fee_claim_public_key;
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

    pub fn with_rocks_override_path<P: AsRef<Path>>(&mut self, path: Option<P>) -> &mut Self {
        self.rocks_override_path = path.as_ref().map(|p| p.as_ref().to_path_buf());
        self
    }

    pub fn with_leader_strategy(&mut self, leader_strategy: RoundRobinLeaderStrategy) -> &mut Self {
        self.leader_strategy = leader_strategy;
        self
    }

    pub fn with_send_observer(&mut self, send_observer: Option<NetworkSendObserver>) -> &mut Self {
        self.send_observer = send_observer;
        self
    }

    pub fn with_num_committees(&mut self, num_committees: u32) -> &mut Self {
        self.num_committees = num_committees;
        self
    }

    /// The store keeps the state history that consensus's down proof lookback reaches into.
    fn database_options(&self) -> DatabaseOptions {
        let state_history_length = self
            .config
            .as_ref()
            .expect("No config given (use ValidatorBuilder::with_config)")
            .state_history_length;
        DatabaseOptions::default()
            .with_debugging_data(true)
            .with_state_history_length(state_history_length)
    }

    #[allow(clippy::too_many_lines)]
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
        let (tx_malformed_message, rx_malformed_message) = mpsc::channel(100);

        let epoch_manager = self.epoch_manager.as_ref().unwrap().clone_for(
            self.address.clone(),
            self.public_key.clone(),
            self.shard_address,
            self.fee_claim_public_key,
        );

        let store = {
            let rocks_path = self
                .rocks_override_path
                .as_deref()
                .unwrap_or_else(|| self.rocks_tmp_path.path());
            log::info!("Rocksdb path {}", rocks_path.display());
            TestStore::open(rocks_path, self.database_options()).unwrap()
        };

        let observer: Option<SendObserver> = self.send_observer.clone().map(|observe| {
            let address = self.address.clone();
            let store = store.clone();
            Arc::new(move |message: &_| observe(&address, &store, message)) as SendObserver
        });
        let (outbound_messaging, rx_loopback) =
            TestOutboundMessaging::create(epoch_manager.clone(), tx_leader, tx_broadcast, observer);
        let inbound_messaging =
            TestInboundMessaging::new(self.address.clone(), rx_hs_message, rx_loopback, rx_malformed_message);

        // Add XTR to the store, since this is implicit for all transactions.
        let (addr, xtr) = tari_ootle_app_utilities::genesis_resources::get_stealth_tari_resource(
            self.config.as_ref().unwrap().network,
        );
        store.with_write_tx(|tx| create_substate(tx, addr, xtr)).unwrap();
        let signing_service = TestVoteSignatureService::new(self.address.clone());
        let transaction_pool = TransactionPool::new();
        let (tx_events, _) = broadcast::channel(100);

        let transaction_executor = TestBlockTransactionProcessor::new(
            self.address.clone(),
            self.transaction_executions.clone(),
            store.clone(),
        );

        let worker = HotstuffWorker::<TestConsensusSpec>::new(
            self.config
                .clone()
                .expect("No config given (use ValidatorBuilder::with_config)"),
            self.address.clone(),
            inbound_messaging,
            outbound_messaging,
            rx_new_transactions,
            store.clone(),
            epoch_manager.clone(),
            self.leader_strategy,
            signing_service,
            transaction_pool,
            transaction_executor.clone(),
            transaction_executor.clone(),
            tx_events.clone(),
            NoopHooks,
            shutdown_signal.clone(),
        );

        let current_view = worker.pacemaker().current_view().clone();

        let (tx_current_state, rx_current_state) = watch::channel(ConsensusCurrentState::default());
        let sync_manager = TestSyncManager::default();
        let context = ConsensusWorkerContext {
            epoch_manager: epoch_manager.clone(),
            hotstuff: worker,
            state_sync: sync_manager.clone(),
            tx_current_state: tx_current_state.clone(),
        };

        let mut worker = ConsensusWorker::new(shutdown_signal).no_initial_delay();
        let handle = tokio::spawn(async move { worker.run(context).await });

        let tx_inbound_message = tx_hs_message.clone();
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
            transaction_executor,
            state_store: store,
            _current_view: current_view,
            epoch_manager,
            events: tx_events.subscribe(),
            current_state_machine_state: rx_current_state,
            sync_manager,
            tx_inbound_message,
            tx_malformed_message,
            handle,
        };
        (channels, validator)
    }
}

fn create_substate<TTx, TId, TVal>(tx: &mut TTx, substate_id: TId, value: TVal) -> Result<(), StorageError>
where
    TTx: StateStoreWriteTransaction + Deref,
    TTx::Target: StateStoreReadTransaction,
    TTx::Addr: NodeAddressable + Serialize,
    TId: Into<SubstateId>,
    TVal: Into<SubstateValue>,
{
    let substate_id = substate_id.into();
    let shard = VersionedSubstateIdRef::new(&substate_id, SubstateVersion::ZERO).to_shard(TEST_NUM_PRESHARDS);
    let mut batch = SubstateUpdateBatch::new(Network::LocalNet, Epoch::zero());
    batch.with_transition(shard, 0).push(SubstateTransition::Up {
        id: substate_id,
        version: SubstateVersion::ZERO,
        substate_or_hash: value.into().into(),
    });

    SubstateRecord::commit_batch(tx, batch)?;

    Ok(())
}
