//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{hash_map, HashMap, HashSet},
    time::Duration,
};

use futures::{stream::FuturesUnordered, FutureExt, StreamExt};
use tari_common_types::types::{PrivateKey, PublicKey};
use tari_consensus::hotstuff::HotstuffEvent;
use tari_crypto::keys::{PublicKey as _, SecretKey};
use tari_dan_common_types::{committee::Committee, shard::Shard, Epoch, NodeHeight};
use tari_dan_storage::{
    consensus_models::{Block, BlockId, Decision, QcId, SubstateRecord, TransactionRecord},
    StateStore,
    StorageError,
};
use tari_engine_types::{
    component::{ComponentBody, ComponentHeader},
    substate::{SubstateId, SubstateValue},
};
use tari_epoch_manager::EpochManagerReader;
use tari_shutdown::{Shutdown, ShutdownSignal};
use tari_template_lib::models::ComponentAddress;
use tari_transaction::{TransactionId, VersionedSubstateId};
use tokio::{sync::broadcast, task, time::sleep};

use super::MessageFilter;
use crate::support::{
    address::TestAddress,
    epoch_manager::TestEpochManager,
    executions_store::TestTransactionExecutionsStore,
    network::{spawn_network, TestNetwork, TestNetworkDestination},
    transaction::build_transaction,
    validator::Validator,
    RoundRobinLeaderStrategy,
    ValidatorChannels,
};

pub struct Test {
    validators: HashMap<TestAddress, Validator>,
    network: TestNetwork,
    transaction_executions: TestTransactionExecutionsStore,
    _leader_strategy: RoundRobinLeaderStrategy,
    epoch_manager: TestEpochManager,
    shutdown: Shutdown,
    timeout: Option<Duration>,
}

impl Test {
    pub fn builder() -> TestBuilder {
        TestBuilder::new()
    }

    pub async fn send_transaction_to(&self, addr: &TestAddress, decision: Decision, fee: u64, num_shards: usize) {
        let num_committees = self.epoch_manager.get_num_committees(Epoch(0)).await.unwrap();
        let transaction = build_transaction(decision, fee, num_shards, num_committees);
        self.send_transaction_to_destination(TestNetworkDestination::Address(addr.clone()), transaction)
            .await;
    }

    pub async fn send_transaction_to_all(&self, decision: Decision, fee: u64, num_shards: usize) {
        let num_committees = self.epoch_manager.get_num_committees(Epoch(0)).await.unwrap();
        let transaction = build_transaction(decision, fee, num_shards, num_committees);
        self.send_transaction_to_destination(TestNetworkDestination::All, transaction)
            .await;
    }

    pub async fn send_transaction_to_destination(&self, dest: TestNetworkDestination, transaction: TransactionRecord) {
        let num_committees = self.epoch_manager.get_num_committees(Epoch(0)).await.unwrap();
        self.validators.values().for_each(|v| {
            if dest.is_for(&v.address, v.substate_address.to_shard(num_committees)) {
                v.state_store.with_write_tx(|tx| transaction.insert(tx)).unwrap();
            }
        });
        self.network.send_transaction(dest, transaction).await;
    }

    pub fn transaction_executions(&self) -> &TestTransactionExecutionsStore {
        &self.transaction_executions
    }

    pub fn create_substates_on_all_vns(&self, num: usize) -> Vec<VersionedSubstateId> {
        assert!(
            num <= u8::MAX as usize,
            "Creating more than 255 substates is not supported"
        );

        let substates = (0..num)
            .map(|i| {
                let id = SubstateId::Component(ComponentAddress::from_array([i as u8; 28]));
                let value = SubstateValue::Component(ComponentHeader {
                    template_address: Default::default(),
                    module_name: "Test".to_string(),
                    owner_key: None,
                    owner_rule: Default::default(),
                    access_rules: Default::default(),
                    entity_id: id.as_component_address().unwrap().entity_id(),
                    body: ComponentBody {
                        state: tari_bor::Value::Null,
                    },
                });
                SubstateRecord::new(
                    id,
                    0,
                    value,
                    Epoch(0),
                    NodeHeight(0),
                    BlockId::genesis(),
                    TransactionId::default(),
                    QcId::genesis(),
                )
            })
            .collect::<Vec<_>>();

        let ids = substates
            .iter()
            .map(|s| VersionedSubstateId::new(s.substate_id().clone(), s.version()))
            .collect::<Vec<_>>();

        self.validators.values().for_each(|v| {
            v.state_store
                .with_write_tx(|tx| {
                    for substate in substates.clone() {
                        substate.create(tx).unwrap();
                    }
                    Ok::<_, StorageError>(())
                })
                .unwrap();
        });

        ids
    }

    pub fn validators(&self) -> hash_map::Values<'_, TestAddress, Validator> {
        self.validators.values()
    }

    pub async fn on_hotstuff_event(&mut self) -> (TestAddress, HotstuffEvent) {
        self.validators
            .values_mut()
            .map(|v| {
                let address = v.address.clone();
                v.events.recv().map(|v| (address, v.unwrap()))
            })
            .collect::<FuturesUnordered<_>>()
            .next()
            .await
            .unwrap()
    }

    pub async fn on_block_committed(&mut self) -> (TestAddress, BlockId, NodeHeight) {
        loop {
            let (address, event) = if let Some(timeout) = self.timeout {
                tokio::time::timeout(timeout, self.on_hotstuff_event())
                    .await
                    .unwrap_or_else(|_| panic!("Timeout waiting for Hotstuff event"))
            } else {
                self.on_hotstuff_event().await
            };
            match event {
                HotstuffEvent::BlockCommitted { block_id, height } => return (address, block_id, height),
                HotstuffEvent::Failure { message } => panic!("[{}] Consensus failure: {}", address, message),
                other => {
                    log::info!("[{}] Ignoring event: {:?}", address, other);
                    continue;
                },
            }
        }
    }

    pub fn network(&mut self) -> &mut TestNetwork {
        &mut self.network
    }

    pub async fn start_epoch(&mut self, epoch: Epoch) {
        for validator in self.validators.values() {
            // Fire off initial epoch change event so that the pacemaker starts
            validator.epoch_manager.set_current_epoch(epoch).await;
        }

        self.wait_for_all_validators_to_start_consensus().await;

        self.network.start();
    }

    pub async fn wait_for_all_validators_to_start_consensus(&mut self) {
        let mut complete = HashSet::new();
        let total_validators = self.validators.len();
        loop {
            let validators = self.validators.values_mut();
            for validator in validators {
                if complete.contains(&validator.address) {
                    continue;
                }

                if validator.current_state_machine_state().is_running() {
                    complete.insert(validator.address.clone());
                    log::info!("Validator {}: consensus is running", validator.address);
                    if complete.len() == total_validators {
                        return;
                    }
                }
            }
            sleep(Duration::from_millis(250)).await;
        }
    }

    #[allow(dead_code)]
    pub fn get_validator_mut(&mut self, addr: &TestAddress) -> &mut Validator {
        self.validators.get_mut(addr).unwrap()
    }

    pub fn get_validator(&self, addr: &TestAddress) -> &Validator {
        self.validators
            .get(addr)
            .unwrap_or_else(|| panic!("No validator with address {}", addr))
    }

    pub fn is_transaction_pool_empty(&self) -> bool {
        self.validators.values().all(|v| {
            let c = v.get_transaction_pool_count();
            log::info!("{} has {} transactions in pool", v.address, c);
            c == 0
        })
    }

    pub fn with_all_validators(&self, f: impl FnMut(&Validator)) {
        self.validators.values().for_each(f);
    }

    #[allow(dead_code)]
    async fn wait_all_for_predicate<P: FnMut(&Validator) -> bool>(&self, description: String, mut predicate: P) {
        let mut complete = vec![];
        let mut remaining_loops = 100usize; // ~10 seconds
        loop {
            self.with_all_validators(|v| {
                if complete.contains(&v.address) {
                    return;
                }
                if predicate(v) {
                    complete.push(v.address.clone());
                } else if remaining_loops == 0 {
                    panic!("Timed out waiting for {}", description);
                } else {
                    // 📎
                }
            });

            remaining_loops -= 1;

            if complete.len() == self.validators.len() {
                break;
            }
            if self.network.task_handle().is_finished() {
                panic!("Network task exited while waiting for {}", description);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    pub async fn assert_all_validators_at_same_height(&self) {
        self.assert_all_validators_at_same_height_except(&[]).await;
    }

    pub async fn assert_all_validators_at_same_height_except(&self, except: &[TestAddress]) {
        let committees = self.epoch_manager.all_committees().await;
        let mut attempts = 0usize;
        'outer: loop {
            for committee in committees.values() {
                let mut heights = self
                    .validators
                    .values()
                    .filter(|vn| committee.contains(&vn.address))
                    .filter(|vn| !except.contains(&vn.address))
                    .map(|v| {
                        let height = v.state_store.with_read_tx(|tx| Block::get_tip(tx)).unwrap().height();
                        (v.address.clone(), height)
                    });
                let (first_addr, first) = heights.next().unwrap();
                for (addr, height) in heights {
                    if first != height && attempts < 5 {
                        attempts += 1;
                        // Send this task to the back of the queue and try again after other tasks have executed
                        // to allow validators to catch up
                        task::yield_now().await;
                        continue 'outer;
                    }
                    assert_eq!(
                        first, height,
                        "Validator {} is at height {} but validator {} is at height {}",
                        first_addr, first, addr, height
                    );
                }
            }
            break;
        }
    }

    pub async fn assert_all_validators_have_decision(
        &self,
        transaction_id: &TransactionId,
        expected_decision: Decision,
    ) {
        let mut attempts = 0usize;
        'outer: loop {
            let decisions = self.validators.values().map(|v| {
                let decisions = v
                    .state_store
                    .with_read_tx(|tx| TransactionRecord::get(tx, transaction_id))
                    .unwrap()
                    .final_decision();
                (v.address.clone(), decisions)
            });
            for (addr, decision) in decisions {
                if decision.is_none() && attempts < 5 {
                    attempts += 1;
                    // Send this task to the back of the queue and try again after other tasks have executed
                    // to allow validators to catch up
                    task::yield_now().await;
                    continue 'outer;
                }

                let decision = decision.unwrap_or_else(|| panic!("VN {} did not make a decision in time", addr));
                assert_eq!(
                    decision, expected_decision,
                    "Expected {} but validator {} has decision {:?} for transaction {}",
                    expected_decision, addr, decision, transaction_id
                );
            }
            break;
        }
    }

    pub fn assert_all_validators_committed(&self) {
        self.validators.values().for_each(|v| {
            assert!(v.has_committed_substates(), "Validator {} did not commit", v.address);
        });
    }

    pub fn assert_all_validators_did_not_commit(&self) {
        self.validators.values().for_each(|v| {
            assert!(
                !v.has_committed_substates(),
                "Validator {} committed but we expected it not to",
                v.address
            );
        });
    }

    pub async fn assert_clean_shutdown(mut self) {
        self.shutdown.trigger();
        for v in self.validators.into_values() {
            v.handle.await.unwrap();
        }
    }
}

pub struct TestBuilder {
    committees: HashMap<Shard, Committee<TestAddress>>,
    sql_address: String,
    timeout: Option<Duration>,
    debug_sql_file: Option<String>,
    message_filter: Option<MessageFilter>,
}

impl TestBuilder {
    pub fn new() -> Self {
        Self {
            committees: HashMap::new(),
            sql_address: ":memory:".to_string(),
            timeout: Some(Duration::from_secs(10)),
            debug_sql_file: None,
            message_filter: None,
        }
    }

    #[allow(dead_code)]
    pub fn disable_timeout(mut self) -> Self {
        self.timeout = None;
        self
    }

    #[allow(dead_code)]
    pub fn debug_sql<P: Into<String>>(mut self, path: P) -> Self {
        self.debug_sql_file = Some(path.into());
        self
    }

    #[allow(dead_code)]
    pub fn with_sql_url<T: Into<String>>(mut self, sql_address: T) -> Self {
        self.sql_address = sql_address.into();
        self
    }

    pub fn with_test_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    pub fn add_committee<T: Into<Shard>>(mut self, bucket: T, addresses: Vec<&'static str>) -> Self {
        let entry = self
            .committees
            .entry(bucket.into())
            .or_insert_with(|| Committee::new(vec![]));

        for addr in addresses {
            let mut bytes = [0u8; 64];
            bytes[0..addr.as_bytes().len()].copy_from_slice(addr.as_bytes());
            let secret_key = PrivateKey::from_uniform_bytes(&bytes).unwrap();
            entry
                .members
                .push((TestAddress::new(addr), PublicKey::from_secret_key(&secret_key)));
        }
        self
    }

    pub fn with_message_filter(mut self, message_filter: MessageFilter) -> Self {
        self.message_filter = Some(message_filter);
        self
    }

    async fn build_validators(
        &self,
        leader_strategy: &RoundRobinLeaderStrategy,
        epoch_manager: &TestEpochManager,
        transaction_executions: TestTransactionExecutionsStore,
        shutdown_signal: ShutdownSignal,
    ) -> (Vec<ValidatorChannels>, HashMap<TestAddress, Validator>) {
        epoch_manager
            .all_validators()
            .await
            .into_iter()
            .map(|(address, bucket, shard, pk, _, _, _)| {
                let sql_address = self.sql_address.replace("{}", &address.0);
                let (channels, validator) = Validator::builder()
                    .with_sql_url(sql_address)
                    .with_transaction_executions(transaction_executions.clone())
                    .with_address_and_public_key(address.clone(), pk.clone())
                    .with_shard(shard)
                    .with_bucket(bucket)
                    .with_epoch_manager(epoch_manager.clone_for(address.clone(), pk, shard))
                    .with_leader_strategy(*leader_strategy)
                    .spawn(shutdown_signal.clone());
                (channels, (address, validator))
            })
            .unzip()
    }

    pub async fn start(mut self) -> Test {
        if let Some(ref sql_file) = self.debug_sql_file {
            // Delete any previous database files
            for path in self
                .committees
                .values()
                .flat_map(|committee| committee.iter().map(|(addr, _)| sql_file.replace("{}", &addr.0)))
            {
                let _ignore = std::fs::remove_file(&path);
            }

            self.sql_address = format!("sqlite://{sql_file}");
        }

        let leader_strategy = RoundRobinLeaderStrategy::new();
        let (tx_epoch_events, _) = broadcast::channel(10);
        let epoch_manager = TestEpochManager::new(tx_epoch_events);
        epoch_manager.add_committees(self.committees.clone()).await;
        let shutdown = Shutdown::new();
        let transaction_executions = TestTransactionExecutionsStore::new();
        let (channels, validators) = self
            .build_validators(
                &leader_strategy,
                &epoch_manager,
                transaction_executions.clone(),
                shutdown.to_signal(),
            )
            .await;
        let network = spawn_network(channels, shutdown.to_signal(), self.message_filter);

        Test {
            validators,
            network,
            transaction_executions,

            _leader_strategy: leader_strategy,
            epoch_manager,
            shutdown,
            timeout: self.timeout,
        }
    }
}
