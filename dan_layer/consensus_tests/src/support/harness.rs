//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{hash_map, HashMap, HashSet},
    fmt::Display,
    time::Duration,
};

use futures::{stream::FuturesUnordered, FutureExt, StreamExt};
use itertools::Itertools;
use log::info;
use tari_common::configuration::Network;
use tari_consensus::{
    consensus_constants::ConsensusConstants,
    hotstuff::{HotstuffConfig, HotstuffEvent},
};
use tari_dan_common_types::{
    committee::Committee,
    shard::Shard,
    Epoch,
    NodeHeight,
    NumPreshards,
    ShardGroup,
    VersionedSubstateId,
};
use tari_dan_storage::{
    consensus_models::{BlockId, Decision, QcId, SubstateRecord, SubstateRequirementLockIntent, TransactionRecord},
    StateStore,
    StateStoreReadTransaction,
    StorageError,
};
use tari_engine_types::substate::SubstateId;
use tari_epoch_manager::EpochManagerReader;
use tari_shutdown::{Shutdown, ShutdownSignal};
use tari_transaction::TransactionId;
use tokio::{sync::broadcast, task, time::sleep};

use super::{
    build_substate_id_for_committee,
    build_transaction,
    helpers,
    random_substates_ids_for_committee_generator,
    MessageFilter,
    TEST_NUM_PRESHARDS,
};
use crate::support::{
    address::TestAddress,
    epoch_manager::TestEpochManager,
    executions_store::ExecuteSpec,
    helpers::make_test_component,
    network::{spawn_network, TestNetwork, TestVnDestination},
    validator::Validator,
    RoundRobinLeaderStrategy,
    ValidatorChannels,
};

pub struct Test {
    validators: HashMap<TestAddress, Validator>,
    network: TestNetwork,
    _leader_strategy: RoundRobinLeaderStrategy,
    epoch_manager: TestEpochManager,
    num_committees: u32,
    shutdown: Shutdown,
    timeout: Option<Duration>,
}

impl Test {
    pub fn builder() -> TestBuilder {
        TestBuilder::new()
    }

    pub async fn send_transaction_to_all(
        &self,
        decision: Decision,
        fee: u64,
        num_inputs: usize,
        num_new_outputs: usize,
    ) -> (TransactionRecord, Vec<VersionedSubstateId>, Vec<SubstateId>) {
        let (transaction, inputs) = self.build_transaction(decision, num_inputs);

        let new_outputs = (0..num_new_outputs)
            .map(|i| build_substate_id_for_committee(i as u32 % self.num_committees, self.num_committees))
            .collect::<Vec<_>>();

        self.add_execution_at_destination(TestVnDestination::All, ExecuteSpec {
            transaction: transaction.transaction().clone(),
            decision,
            fee,
            inputs: inputs
                .iter()
                .map(|input| SubstateRequirementLockIntent::write(input.clone(), input.version()))
                .collect(),
            new_outputs: new_outputs.clone(),
        });

        self.send_transaction_to_destination(TestVnDestination::All, transaction.clone())
            .await;

        (transaction, inputs, new_outputs)
    }

    pub async fn send_transaction_to_destination(&self, dest: TestVnDestination, transaction: TransactionRecord) {
        self.network.send_transaction(dest, transaction).await;
    }

    pub fn add_execution_at_destination(&self, dest: TestVnDestination, execution: ExecuteSpec) -> &Self {
        for vn in self.validators.values() {
            if dest.is_for(&vn.address, vn.shard_group, vn.num_committees) {
                vn.transaction_executions.insert(execution.clone());
            }
        }
        self
    }

    pub fn create_execution_at_destination_for_transaction(
        &self,
        dest: TestVnDestination,
        transaction: &TransactionRecord,
        inputs: Vec<SubstateRequirementLockIntent>,
        new_outputs: Vec<SubstateId>,
    ) -> &Self {
        self.add_execution_at_destination(dest, ExecuteSpec {
            transaction: transaction.transaction().clone(),
            decision: transaction.current_decision(),
            fee: transaction.transaction_fee().unwrap_or(0),
            inputs,
            new_outputs,
        });
        self
    }

    pub fn build_transaction(
        &self,
        decision: Decision,
        num_inputs: usize,
    ) -> (TransactionRecord, Vec<VersionedSubstateId>) {
        let all_inputs = self.create_substates_on_vns(TestVnDestination::All, num_inputs);
        let transaction = build_transaction(decision, all_inputs.iter().cloned().map(Into::into).collect());
        (transaction, all_inputs)
    }

    pub fn create_substates_on_vns(&self, dest: TestVnDestination, num: usize) -> Vec<VersionedSubstateId> {
        assert!(
            num <= u8::MAX as usize,
            "Creating more than 255 substates is not supported"
        );

        let substate_ids = match dest {
            TestVnDestination::All => (0..self.num_committees)
                .flat_map(|committee_no| {
                    random_substates_ids_for_committee_generator(committee_no, self.num_committees).take(num)
                })
                .map(|id| VersionedSubstateId::new(id, 0))
                .collect::<Vec<_>>(),
            TestVnDestination::Address(_) => unimplemented!(
                "Creating substates for a specific validator is not supported as it isn't typically useful"
            ),
            TestVnDestination::Committee(committee_no) => {
                random_substates_ids_for_committee_generator(committee_no, self.num_committees)
                    .take(num)
                    .map(|id| VersionedSubstateId::new(id, 0))
                    .collect::<Vec<_>>()
            },
        };

        let substates = substate_ids
            .iter()
            .map(|id| {
                let value = make_test_component(id.substate_id().as_component_address().unwrap().entity_id());
                SubstateRecord::new(
                    id.substate_id.clone(),
                    id.version,
                    value,
                    Shard::zero(),
                    Epoch(0),
                    NodeHeight(0),
                    BlockId::zero(),
                    TransactionId::default(),
                    QcId::zero(),
                )
            })
            .collect::<Vec<_>>();

        self.validators.values().filter(|vn| dest.is_for_vn(vn)).for_each(|v| {
            v.state_store
                .with_write_tx(|tx| {
                    for substate in &substates {
                        if v.shard_group
                            .contains(&substate.to_substate_address().to_shard(TEST_NUM_PRESHARDS))
                        {
                            substate.create(tx).unwrap();
                        }
                    }
                    Ok::<_, StorageError>(())
                })
                .unwrap();
        });

        substate_ids
    }

    pub fn build_outputs_for_committee(&self, committee_no: u32, num_outputs: usize) -> Vec<SubstateId> {
        random_substates_ids_for_committee_generator(committee_no, self.num_committees)
            .take(num_outputs)
            .collect()
    }

    pub fn validators_iter(&self) -> hash_map::Values<'_, TestAddress, Validator> {
        self.validators.values()
    }

    pub fn validators(&self) -> &HashMap<TestAddress, Validator> {
        &self.validators
    }

    pub async fn on_hotstuff_event(&mut self) -> (TestAddress, HotstuffEvent) {
        if self.network.task_handle().is_finished() {
            panic!("Network task exited while waiting for Hotstuff event");
        }
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

    pub async fn on_block_committed(&mut self) -> (TestAddress, BlockId, Epoch, NodeHeight) {
        loop {
            let (address, event) = if let Some(timeout) = self.timeout {
                match tokio::time::timeout(timeout, self.on_hotstuff_event()).await {
                    Ok(v) => v,
                    Err(_) => {
                        self.dump_pool_info();
                        panic!("Timeout waiting for Hotstuff event");
                    },
                }
            } else {
                self.on_hotstuff_event().await
            };
            if self.network.is_offline(&address, self.num_committees).await {
                log::info!("[{}] Ignoring event for offline node: {:?}", address, event);
                continue;
            }

            match event {
                HotstuffEvent::BlockCommitted {
                    block_id,
                    epoch,
                    height,
                } => return (address, block_id, epoch, height),
                HotstuffEvent::Failure { message } => panic!("[{}] Consensus failure: {}", address, message),
                other => {
                    log::info!("[{}] Ignoring event: {:?}", address, other);
                    continue;
                },
            }
        }
    }

    pub fn dump_pool_info(&self) {
        for v in self.validators.values().sorted_unstable_by_key(|a| &a.address) {
            let pool = v.state_store.with_read_tx(|tx| tx.transaction_pool_get_all()).unwrap();
            for tx in pool {
                eprintln!(
                    "{}: {}->{:?} {}[{}, ready={}, {}]",
                    v.address,
                    tx.current_stage(),
                    tx.pending_stage(),
                    tx.transaction_id(),
                    tx.current_decision(),
                    tx.is_ready(),
                    tx.evidence()
                );
            }
        }
    }

    pub fn network(&mut self) -> &mut TestNetwork {
        &mut self.network
    }

    pub async fn start_epoch(&mut self, epoch: Epoch) {
        info!("🌟 Starting {epoch}");
        for validator in self.validators.values_mut() {
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

    pub async fn wait_for_n_to_be_finalized(&self, n: usize) {
        self.wait_all_for_predicate("waiting for n to be finalized", |vn| {
            let transactions = vn
                .state_store
                .with_read_tx(|tx| tx.transactions_get_paginated(10000, 0, None))
                .unwrap();
            log::info!("{} has {} transactions in pool", vn.address, transactions.len());
            transactions.iter().filter(|tx| tx.is_finalized()).count() >= n
        })
        .await
    }

    pub async fn wait_for_pool_count(&self, dest: TestVnDestination, count: usize) {
        self.wait_all_for_predicate("waiting for pool count", |vn| {
            if !dest.is_for_vn(vn) {
                return true;
            }
            let c = vn.get_transaction_pool_count();
            log::info!("{} has {} transactions in pool", vn.address, c);
            c >= count
        })
        .await
    }

    pub fn with_all_validators(&self, f: impl FnMut(&Validator)) {
        self.validators.values().for_each(f);
    }

    async fn wait_all_for_predicate<T: Display, P: FnMut(&Validator) -> bool>(&self, description: T, mut predicate: P) {
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
        let current_epoch = self.epoch_manager.current_epoch().await.unwrap();
        let committees = self.epoch_manager.all_committees().await;
        let mut attempts = 0usize;
        'outer: loop {
            for (shard_group, committee) in &committees {
                let mut blocks = self
                    .validators
                    .values()
                    .filter(|vn| committee.contains(&vn.address))
                    .filter(|vn| !except.contains(&vn.address))
                    .map(|v| {
                        let block = v
                            .state_store
                            .with_read_tx(|tx| tx.blocks_get_tip(current_epoch, *shard_group))
                            .unwrap();
                        (v.address.clone(), block)
                    });
                let (first_addr, first) = blocks.next().unwrap();
                for (addr, block) in blocks {
                    if (first.epoch() != block.epoch() || first.height() != block.height()) && attempts < 5 {
                        attempts += 1;
                        // Send this task to the back of the queue and try again after other tasks have executed
                        // to allow validators to catch up
                        task::yield_now().await;
                        continue 'outer;
                    }
                    assert_eq!(
                        first.id(),
                        block.id(),
                        "Validator {} is at height {} but validator {} is at height {}",
                        first_addr,
                        first,
                        addr,
                        block
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
                    .unwrap_or_else(|err| panic!("{} Error getting transaction {}: {}", v.address, transaction_id, err))
                    .final_decision();
                (v.address.clone(), decisions)
            });
            for (addr, decision) in decisions {
                if decision.is_none() && attempts < 5 {
                    attempts += 1;
                    // Send this task to the back of the queue and try again after other tasks have executed
                    // to allow validators to catch up
                    // tokio::time::sleep(Duration::from_millis(50)).await;
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

    pub async fn assert_clean_shutdown(&mut self) {
        self.shutdown.trigger();
        for (_, v) in self.validators.drain() {
            v.handle.await.unwrap();
        }
    }

    pub async fn assert_clean_shutdown_except(&mut self, except: &[TestAddress]) {
        self.shutdown.trigger();
        for (_, v) in self.validators.drain() {
            if !except.contains(&v.address) {
                v.handle.await.unwrap();
            }
        }
    }
}

pub struct TestBuilder {
    committees: HashMap<u32, Committee<TestAddress>>,
    sql_address: String,
    timeout: Option<Duration>,
    debug_sql_file: Option<String>,
    message_filter: Option<MessageFilter>,
    failure_nodes: Vec<TestAddress>,
    config: HotstuffConfig,
}

impl TestBuilder {
    pub fn new() -> Self {
        Self {
            committees: HashMap::new(),
            sql_address: ":memory:".to_string(),
            timeout: Some(Duration::from_secs(10)),
            debug_sql_file: None,
            message_filter: None,
            failure_nodes: Vec::new(),
            config: HotstuffConfig {
                network: Network::LocalNet,
                sidechain_id: None,
                consensus_constants: ConsensusConstants {
                    base_layer_confirmations: 0,
                    committee_size: 10,
                    max_base_layer_blocks_ahead: 5,
                    max_base_layer_blocks_behind: 5,
                    num_preshards: TEST_NUM_PRESHARDS,
                    pacemaker_block_time: Duration::from_secs(10),
                    missed_proposal_suspend_threshold: 5,
                    missed_proposal_count_cap: 5,
                    max_block_size: 500,
                    fee_exhaust_divisor: 20,
                    max_vns_per_epoch_activated: 5,
                },
            },
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

    pub fn add_committee(mut self, committee_num: u32, addresses: Vec<&'static str>) -> Self {
        let entry = self
            .committees
            .entry(committee_num)
            .or_insert_with(|| Committee::new(vec![]));

        for addr in addresses {
            let addr = TestAddress::new(addr);
            let (_, pk) = helpers::derive_keypair_from_address(&addr);
            entry.members.push((addr, pk));
        }
        self
    }

    pub fn add_failure_node<T: Into<TestAddress>>(mut self, node: T) -> Self {
        self.failure_nodes.push(node.into());
        self
    }

    pub fn with_message_filter(mut self, message_filter: MessageFilter) -> Self {
        self.message_filter = Some(message_filter);
        self
    }

    pub fn modify_consensus_constants<F: FnOnce(&mut ConsensusConstants)>(mut self, f: F) -> Self {
        f(&mut self.config.consensus_constants);
        self
    }

    async fn build_validators(
        leader_strategy: &RoundRobinLeaderStrategy,
        epoch_manager: &TestEpochManager,
        sql_address: String,
        config: HotstuffConfig,
        failure_nodes: &[TestAddress],
        shutdown_signal: ShutdownSignal,
    ) -> (Vec<ValidatorChannels>, HashMap<TestAddress, Validator>) {
        let num_committees = epoch_manager.get_num_committees(Epoch(0)).await.unwrap();
        epoch_manager
            .all_validators()
            .await
            .into_iter()
            // Dont start failed nodes
            .filter(|(addr, _, _, pk, _, _, _)| {
                if failure_nodes.contains(addr) {
                    log::info!("❗️ {addr} {pk} is a failure node and will not be spawned");
                    return false;
                }
                true
            })
            .map(|(address, shard_group, shard_addr, _, _, _, _)| {
                let sql_address = sql_address.replace("{}", &address.0);
                let (sk, pk) = helpers::derive_keypair_from_address(&address);

                let (channels, validator) = Validator::builder()
                    .with_sql_url(sql_address)
                    .with_config(config.clone())
                    .with_address_and_secret_key(address.clone(), sk)
                    .with_shard(shard_addr)
                    .with_shard_group(shard_group)
                    .with_epoch_manager(epoch_manager.clone_for(address.clone(), pk, shard_addr))
                    .with_leader_strategy(*leader_strategy)
                    .with_num_committees(num_committees)
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

        let committees = build_committees(self.committees);
        let num_committees = u32::try_from(committees.len()).expect("WAAAY too many committees");

        let leader_strategy = RoundRobinLeaderStrategy::new();
        let (tx_epoch_events, _) = broadcast::channel(10);
        let epoch_manager = TestEpochManager::new(tx_epoch_events);
        epoch_manager.add_committees(committees).await;
        let shutdown = Shutdown::new();
        let (channels, validators) = Self::build_validators(
            &leader_strategy,
            &epoch_manager,
            self.sql_address,
            self.config,
            &self.failure_nodes,
            shutdown.to_signal(),
        )
        .await;
        let network = spawn_network(channels, shutdown.to_signal(), self.message_filter);

        Test {
            validators,
            network,
            num_committees,

            _leader_strategy: leader_strategy,
            epoch_manager,
            shutdown,
            timeout: self.timeout,
        }
    }
}

/// Converts a test committee number to a shard group. E.g. 0 is shard group 0 to 21, 1 is 22 to 42, etc.
pub fn committee_number_to_shard_group(num_shards: NumPreshards, target_group: u32, num_committees: u32) -> ShardGroup {
    // number of committees can never exceed number of shards
    assert!(num_committees <= num_shards.as_u32());
    if num_committees <= 1 {
        return ShardGroup::new(Shard::zero(), Shard::from(num_shards.as_u32() - 1));
    }

    let shards_per_committee = num_shards.as_u32() / num_committees;
    let mut shards_per_committee_rem = num_shards.as_u32() % num_committees;

    let mut start = 0u32;
    let mut end = shards_per_committee;
    if shards_per_committee_rem > 0 {
        end += 1;
    }

    for _group in 0..target_group {
        start += shards_per_committee;
        if shards_per_committee_rem > 0 {
            start += 1;
            shards_per_committee_rem -= 1;
        }

        end = start + shards_per_committee;
        if shards_per_committee_rem > 0 {
            end += 1;
        }
    }

    ShardGroup::new(start, end - 1)
}

fn build_committees(committees: HashMap<u32, Committee<TestAddress>>) -> HashMap<ShardGroup, Committee<TestAddress>> {
    let num_committees = committees.len() as u32;
    committees
        .into_iter()
        .map(|(num, committee)| {
            let shard_group = committee_number_to_shard_group(TEST_NUM_PRESHARDS, num, num_committees);
            (shard_group, committee)
        })
        .collect()
}
