//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{HashMap, HashSet},
    fmt::Display,
    fs,
    time::Duration,
};

use futures::{FutureExt, StreamExt, stream::FuturesUnordered};
use itertools::Itertools;
use log::info;
use ootle_byte_type::ToByteType;
use tari_consensus::{
    consensus_constants::ConsensusConstants,
    hotstuff::{HotstuffConfig, HotstuffEvent},
};
use tari_consensus_types::{BlockId, Decision};
use tari_crypto::ristretto::RistrettoPublicKey;
use tari_engine_types::{fees::ExhaustBurnRate, substate::SubstateId};
use tari_epoch_manager::EpochManagerReader;
use tari_ootle_common_types::{
    Epoch,
    NodeHeight,
    NumPreshards,
    ShardGroup,
    SubstateLockType,
    VersionedSubstateId,
    VotePower,
    committee::{Committee, CommitteeMember},
    displayable::Displayable,
    optional::Optional,
    shard::Shard,
};
use tari_ootle_storage::{
    Ordering,
    StateStore,
    StateStoreReadTransaction,
    consensus_models::{SubstateCreated, SubstateRecord, SubstateUpdateBatch, TransactionExecution, TransactionRecord},
};
use tari_ootle_transaction::{Network, TransactionId};
use tari_shutdown::{Shutdown, ShutdownSignal};
use tari_state_store_rocksdb::column_families::{
    finalized_transaction::FinalizedTransactionLinkCf,
    transaction::TransactionCf,
};
use tokio::{sync::broadcast, task, time::sleep};

use super::{
    MessageFilter,
    TEST_NUM_PRESHARDS,
    TEST_WASM_EXECUTION_POINTS,
    build_substate_id_for_committee,
    build_transaction,
    helpers,
    random_substates_ids_for_committee_generator,
};
use crate::{
    support::{
        RoundRobinLeaderStrategy,
        ValidatorChannels,
        address::TestAddress,
        epoch_manager::TestEpochManager,
        executions_store::ExecuteSpec,
        helpers::make_test_component,
        network::{TestNetwork, TestVnDestination, spawn_network},
        table::Table,
        validator::Validator,
    },
    table_row,
};

pub struct Test {
    validators: HashMap<TestAddress, Validator>,
    network: TestNetwork,
    _leader_strategy: RoundRobinLeaderStrategy,
    _epoch_manager: TestEpochManager,
    num_committees: u32,
    submitted_transactions: Vec<TransactionId>,
    shutdown: Shutdown,
    timeout: Option<Duration>,
}

impl Test {
    pub fn builder() -> TestBuilder {
        TestBuilder::new()
    }

    pub async fn send_transaction_to_all(
        &mut self,
        decision: Decision,
        fee: u64,
        num_inputs: usize,
        num_new_outputs: usize,
    ) -> (TransactionRecord, Vec<VersionedSubstateId>, Vec<SubstateId>) {
        let (transaction, inputs) = self.build_transaction(num_inputs);

        let new_outputs = (0..num_new_outputs)
            .map(|i| build_substate_id_for_committee(i as u32 % self.num_committees, self.num_committees))
            .collect::<Vec<_>>();

        self.submitted_transactions.push(*transaction.id());

        self.add_execution_at_destination(TestVnDestination::All, ExecuteSpec {
            transaction: transaction.transaction().clone(),
            decision,
            fee,
            input_locks: inputs
                .iter()
                .map(|input| (input.substate_id().clone(), SubstateLockType::Write))
                .collect(),
            new_outputs: new_outputs.clone(),
            validator_fee_withdrawals: vec![],
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
                vn.transaction_executor.store().insert(execution.clone());
            }
        }
        self
    }

    pub fn create_execution_at_destination_for_transaction(
        &self,
        dest: TestVnDestination,
        transaction: &TransactionRecord,
        decision: Decision,
        fee: u64,
        input_locks: Vec<(SubstateId, SubstateLockType)>,
        new_outputs: Vec<SubstateId>,
    ) -> &Self {
        self.add_execution_at_destination(dest, ExecuteSpec {
            transaction: transaction.transaction().clone(),
            decision,
            fee,
            input_locks,
            new_outputs,
            validator_fee_withdrawals: vec![],
        });
        self
    }

    pub fn build_transaction(&self, num_inputs: usize) -> (TransactionRecord, Vec<VersionedSubstateId>) {
        let all_inputs = self.create_substates_on_vns(TestVnDestination::All, num_inputs);
        let transaction = build_transaction(all_inputs.iter().cloned().map(Into::into).collect());
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
                let shard = id.to_shard(TEST_NUM_PRESHARDS);
                SubstateRecord::new(
                    Network::LocalNet,
                    id.substate_id().clone(),
                    id.version(),
                    value,
                    SubstateCreated {
                        at_epoch: Epoch::zero(),
                        in_shard: shard,
                        at_state_version: 0,
                    },
                )
            })
            .collect::<Vec<_>>();

        self.validators.values().filter(|vn| dest.is_for_vn(vn)).for_each(|v| {
            let mut batch = SubstateUpdateBatch::new(Network::LocalNet, Epoch::zero());
            for substate in &substates {
                let shard = substate.to_versioned_substate_id().to_shard(TEST_NUM_PRESHARDS);
                if v.shard_group.contains(&shard) {
                    batch
                        .with_transition(shard, substate.created().at_state_version)
                        .push(substate.clone().into_transition());
                }
            }

            v.state_store
                .with_write_tx(|tx| SubstateRecord::commit_batch(tx, batch))
                .unwrap();
        });

        substate_ids
    }

    pub fn build_outputs_for_committee(&self, committee_no: u32, num_outputs: usize) -> Vec<SubstateId> {
        random_substates_ids_for_committee_generator(committee_no, self.num_committees)
            .take(num_outputs)
            .collect()
    }

    pub fn validators_iter(&self) -> impl Iterator<Item = &Validator> {
        self.validators.values()
    }

    pub fn validators(&self) -> &HashMap<TestAddress, Validator> {
        &self.validators
    }

    pub const fn num_preshards(&self) -> NumPreshards {
        TEST_NUM_PRESHARDS
    }

    #[allow(dead_code)]
    pub fn num_committees(&self) -> u32 {
        self.num_committees
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
                        // We assume there is a "1" (hint: there always is)
                        self.dump_blocks(&TestAddress::new("1"));
                        panic!("Timeout waiting for Hotstuff event");
                    },
                }
            } else {
                self.on_hotstuff_event().await
            };
            if self.network.is_offline(&address).await {
                info!("[{}] Ignoring event for offline node: {:?}", address, event);
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
                    info!("[{}] Ignoring event: {:?}", address, other);
                    continue;
                },
            }
        }
    }

    pub fn dump_pool_info(&self) {
        for v in self.validators.values().sorted_unstable_by_key(|a| &a.address) {
            let pool = v
                .state_store
                .with_read_tx(|tx| tx.transaction_pool_get_all(1000))
                .unwrap();
            println!("Validator {}", v.address);
            let mut table = Table::new();
            table
                .set_titles(vec![
                    "Address",
                    "Stage",
                    "PendingStage",
                    "ID",
                    "Decision",
                    "Ready",
                    "Evidence",
                ])
                .enable_row_count();
            for tx in pool {
                table.add_row(table_row![
                    v.address,
                    tx.current_stage(),
                    tx.pending_stage().display(),
                    tx.id(),
                    tx.current_decision(),
                    tx.is_ready(),
                    format!(
                        "A:{} P:{}",
                        if tx.evidence().all_shard_groups_prepared() {
                            "✅"
                        } else {
                            "❌"
                        },
                        if tx.evidence().all_shard_groups_accepted() {
                            "✅"
                        } else {
                            "❌"
                        }
                    ),
                ]);
            }

            table.print_stdout();
        }
    }

    pub fn dump_blocks(&self, addr: &TestAddress) {
        println!("====================");
        println!("Blocks for {}", addr);
        println!("====================");
        let v = self
            .validators
            .get(addr)
            .unwrap_or_else(|| panic!("dump_blocks: No validator with address {}", addr));
        let blocks = v
            .state_store
            .with_read_tx(|tx| tx.blocks_get_paginated(1000, 0, None, None, None, None))
            .unwrap();
        let mut table = Table::new();
        table
            .set_titles(vec!["Block ID", "Epoch", "Height", "Parent", "Cmds"])
            .enable_row_count();
        for block in blocks {
            table.add_row(table_row![
                block.id(),
                block.epoch().as_u64(),
                block.height().as_u64(),
                format!(
                    "C: {}, J: {}, D: {}",
                    block.is_committed(),
                    block.has_justify_qc(),
                    block.is_dummy()
                ),
                block.commands().display()
            ]);
        }

        table.print_stdout();
    }

    pub fn network(&mut self) -> &mut TestNetwork {
        &mut self.network
    }

    pub fn stop(&mut self) {
        self.network.pause();
        self.shutdown.trigger();
    }

    pub async fn start_epoch(&mut self, epoch: Epoch) {
        info!("🌟 Starting {epoch}");
        for validator in self.validators.values_mut() {
            // Fire off initial epoch change event so that the pacemaker starts
            validator
                .epoch_manager
                .set_current_epoch(epoch, validator.shard_group)
                .await;
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
            if c > 0 {
                log::info!("🐞 {} has {} transactions in pool", v.address, c);
            }
            c == 0
        })
    }

    #[track_caller]
    pub fn is_all_submitted_transactions_finalized(&self) -> bool {
        assert!(
            !self.submitted_transactions.is_empty(),
            "No submitted transactions to check"
        );
        self.validators.values().all(|v| {
            let tx = v.state_store.create_read_tx().unwrap();
            let link_cf = tx.db().cf(FinalizedTransactionLinkCf).unwrap();
            for id in &self.submitted_transactions {
                if !link_cf.exists(id, "test").unwrap() {
                    return false;
                }
            }
            true
        })
    }

    pub fn is_transaction_finalized_at_destination(
        &self,
        destination: TestVnDestination,
        transaction_id: &TransactionId,
    ) -> bool {
        self.validators
            .values()
            .filter(|vn| destination.is_for_vn(vn))
            .all(|vn| {
                let tx = vn.state_store.create_read_tx().unwrap();
                let link_cf = tx.db().cf(FinalizedTransactionLinkCf).unwrap();
                link_cf.exists(transaction_id, "test").unwrap()
            })
    }

    pub async fn wait_for_n_to_be_finalized(&self, n: usize) {
        self.wait_all_for_predicate(format!("waiting for {n} to be finalized"), |vn| {
            let tx = vn.state_store.create_read_tx().unwrap();
            let cf = tx.db().cf(TransactionCf).unwrap();
            let transactions = cf.key_iterator(Ordering::Ascending, "test");
            let link_cf = tx.db().cf(FinalizedTransactionLinkCf).unwrap();
            let mut finalized = 0;
            let mut count = 0;
            for transaction in transactions {
                count += 1;
                if link_cf.exists(&transaction.unwrap(), "test").unwrap() {
                    finalized += 1;
                    if finalized >= n {
                        return true;
                    }
                }
            }
            log::info!("{} has {} transactions", vn.address, count);
            false
        })
        .await
    }

    pub async fn wait_for_pool_count(&self, dest: TestVnDestination, count: usize) {
        self.wait_all_for_predicate("pool count > n", |vn| {
            if !dest.is_for_vn(vn) {
                return true;
            }
            let c = vn.get_transaction_pool_count();
            log::info!("{} has {} transactions in pool", vn.address, c);
            c >= count
        })
        .await
    }

    /// Waits until every validator in `dest` has received the transaction. Unlike waiting on a pool count,
    /// this cannot be raced by the transaction being proposed and finalized between two polls.
    pub async fn wait_for_transaction_seen(&self, dest: TestVnDestination, tx_id: &TransactionId) {
        self.wait_all_for_predicate(format!("transaction {tx_id} seen"), |vn| {
            if !dest.is_for_vn(vn) {
                return true;
            }
            vn.has_seen_transaction(tx_id)
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
                    panic!(
                        "Timed out waiting for {} ({} of {} validators passed)",
                        description,
                        complete.len(),
                        self.validators.len()
                    );
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

    pub async fn assert_all_validators_have_decision(
        &self,
        transaction_id: &TransactionId,
        expected_decision: Decision,
    ) {
        self.assert_validators_have_decision(TestVnDestination::All, transaction_id, expected_decision)
            .await;
    }

    pub async fn assert_validators_have_decision(
        &self,
        dest: TestVnDestination,
        transaction_id: &TransactionId,
        expected_decision: Decision,
    ) {
        let mut attempts = 0usize;
        'outer: loop {
            let decisions = self.validators.values().filter(|vn| dest.is_for_vn(vn)).map(|v| {
                let decision = v
                    .state_store
                    .with_read_tx(|tx| TransactionExecution::get_finalized(tx, transaction_id))
                    .optional()
                    .unwrap_or_else(|err| panic!("{} Error getting transaction {}: {}", v.address, transaction_id, err))
                    .map(|e| e.decision());
                (v.address.clone(), decision)
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

    pub fn assert_all_validators_committed(&self, tx_id: &TransactionId) {
        self.assert_destination_has_committed(TestVnDestination::All, tx_id);
    }

    pub fn assert_destination_has_committed(&self, dest: TestVnDestination, tx_id: &TransactionId) {
        self.validators.values().filter(|vn| dest.is_for_vn(vn)).for_each(|v| {
            assert!(
                v.has_committed_substates(tx_id),
                "Validator {} did not commit transaction {}",
                v.address,
                tx_id
            );
        });
    }

    pub fn assert_all_validators_did_not_commit(&self, tx_id: &TransactionId) {
        self.validators.values().for_each(|v| {
            assert!(
                !v.has_committed_substates(tx_id),
                "Validator {} committed {} but expected it to reject",
                v.address,
                tx_id
            );
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
    rocks_path: Option<String>,
    timeout: Option<Duration>,
    message_filter: Option<MessageFilter>,
    failure_nodes: Vec<TestAddress>,
    config: HotstuffConfig,
    claim_keys: Option<(TestVnDestination, RistrettoPublicKey)>,
    allow_wasm_budget_deferrals: bool,
}

impl TestBuilder {
    pub fn new() -> Self {
        const DEFAULT_PACEMAKER_BLOCK_TIME: Duration = Duration::from_secs(10);
        Self {
            committees: HashMap::new(),
            // By default, timeout aims to occur after we allow enough time for a "slow" block
            timeout: Some(DEFAULT_PACEMAKER_BLOCK_TIME + Duration::from_secs(2)),
            rocks_path: None,
            message_filter: None,
            failure_nodes: Vec::new(),
            claim_keys: None,
            allow_wasm_budget_deferrals: false,
            config: HotstuffConfig {
                network: Network::LocalNet,
                sidechain_id: None,
                consensus_constants: ConsensusConstants {
                    base_layer_confirmations: 0,
                    committee_size_per_shard_group: 10,
                    num_preshards: TEST_NUM_PRESHARDS,
                    pacemaker_block_time: DEFAULT_PACEMAKER_BLOCK_TIME,
                    missed_proposal_suspend_threshold: 5,
                    missed_proposal_evict_threshold: 10,
                    missed_proposal_recovery_threshold: 5,
                    max_transaction_validity_epochs: 100,
                    // Keep the weight budget effectively unbounded in tests so behaviour stays
                    // count-limited (as before) unless a test specifically exercises the weight budget.
                    max_block_weight: 1_000_000,
                    max_commands_in_block: 500,
                    // Effectively disable the validation-time weight cap by default; tests that exercise
                    // it set a low value explicitly.
                    max_block_validation_weight: u64::MAX,
                    // Per-transaction mempool weight cap; effectively unbounded in tests.
                    max_transaction_weight: u64::MAX,
                    // Per-transaction byte cap; likewise unbounded, so tests are shaped by the
                    // behaviour they exercise rather than by transaction size.
                    max_transaction_size_bytes: usize::MAX,
                    // Network-default wasm budgets. Fabricated executions consume
                    // TEST_WASM_EXECUTION_POINTS each, sized so normal test transactions can never
                    // hit the budget (asserted in `start`). Tests exercising excessive computation
                    // lower these explicitly and opt out via `allow_wasm_budget_deferrals`.
                    max_block_execution_points: 4_500_000_000,
                    max_block_validation_execution_points: 5_000_000_000,
                    exhaust_burn_rate: ExhaustBurnRate::new(500),
                    epoch_end_spread_blocks: 0,
                },
                state_tree_cleanup_interval: Duration::from_secs(1000),
                epoch_gc_interval: Duration::from_secs(1000),
                enable_eviction_proposal: true,
                epoch_end_grace_period: Duration::from_secs(1),
                catch_up_request_timeout: Duration::from_secs(15),
            },
        }
    }

    #[allow(dead_code)]
    pub fn disable_timeout(mut self) -> Self {
        self.timeout = None;
        self
    }

    pub fn modify_config<F: FnOnce(&mut HotstuffConfig)>(mut self, f: F) -> Self {
        f(&mut self.config);
        self
    }

    #[allow(dead_code)]
    pub fn with_rocks_path<T: Into<String>>(mut self, path: T) -> Self {
        self.rocks_path = Some(path.into());
        self
    }

    pub fn with_test_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    pub fn set_claim_key(mut self, dest: TestVnDestination, claim_key: RistrettoPublicKey) -> Self {
        self.claim_keys = Some((dest, claim_key));
        self
    }

    pub fn add_committee(mut self, committee_num: u32, addresses: Vec<&'static str>) -> Self {
        let entry = self.committees.entry(committee_num).or_insert_with(Committee::empty);

        for addr in addresses {
            let addr = TestAddress::new(addr);
            let (_, pk) = helpers::derive_keypair_from_address(&addr);
            entry.members_mut().push(CommitteeMember {
                address: addr,
                public_key: pk.to_byte_type(),
                vote_power: VotePower::of(1),
            });
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

    /// Opt out of the wasm budget invariant check in `start` for tests that deliberately exercise
    /// excessive computation (i.e. expect transactions to be deferred by the wasm points budget).
    #[allow(dead_code)]
    pub fn allow_wasm_budget_deferrals(mut self) -> Self {
        self.allow_wasm_budget_deferrals = true;
        self
    }

    async fn build_validators(
        leader_strategy: &RoundRobinLeaderStrategy,
        epoch_manager: &TestEpochManager,
        rocks_path_override: Option<String>,
        config: HotstuffConfig,
        failure_nodes: &[TestAddress],
        shutdown_signal: ShutdownSignal,
    ) -> (Vec<ValidatorChannels>, HashMap<TestAddress, Validator>) {
        let num_committees = epoch_manager.get_num_committees(Epoch(1)).await.unwrap();
        epoch_manager
            .all_validators()
            .await
            .into_iter()
            // Dont start failed nodes
            .filter(|(vn, _)| {
                if failure_nodes.contains(&vn.address) {
                    log::info!("❗️ {} {} is a failure node and will not be spawned", vn.address, vn.public_key);
                    return false;
                }
                true
            })
            .map(|(vn, shard_group)| {
                let rocks_path_override = rocks_path_override
                    .as_ref()
                    .map(|path| path.replace("{}", vn.address.as_str()));

                if let Some(ref rocks_path) = rocks_path_override {
                    if fs::exists(rocks_path).unwrap() {
                        log::info!("Removing existing rocks path {}", rocks_path);
                        let _ignore = fs::remove_dir_all(rocks_path);
                    }
                    fs::create_dir_all(rocks_path).unwrap();
                }
                let (sk, pk) = helpers::derive_keypair_from_address(&vn.address);

                let (channels, validator) = Validator::builder()
                    .with_rocks_override_path(rocks_path_override)
                    .with_config(config.clone())
                    .with_address_and_secret_key(vn.address.clone(), sk)
                    .with_shard(vn.shard_key)
                    .with_shard_group(shard_group)
                    .with_fee_claim_public_key(vn.fee_claim_public_key)
                    .with_epoch_manager(epoch_manager.clone_for(vn.address.clone(), pk, vn.shard_key, vn.fee_claim_public_key))
                    .with_leader_strategy(*leader_strategy)
                    .with_num_committees(num_committees)
                    .spawn(shutdown_signal.clone());
                (channels, (vn.address, validator))
            })
            .unzip()
    }

    pub async fn start(self) -> Test {
        // Normal test transactions must never hit the wasm points budget: even a block filled to
        // max_commands_in_block must fit within the proposer budget. A config that violates this
        // would silently defer transactions and change scheduling; fail loudly here instead.
        if !self.allow_wasm_budget_deferrals {
            let constants = &self.config.consensus_constants;
            let max_points_per_block = constants.max_commands_in_block as u64 * TEST_WASM_EXECUTION_POINTS;
            assert!(
                max_points_per_block <= constants.max_block_execution_points,
                "Test config allows normal transactions to hit the wasm points budget: {} commands × {} points/tx = \
                 {} > max_block_execution_points {}. If this test deliberately exercises excessive computation, opt \
                 out with allow_wasm_budget_deferrals().",
                constants.max_commands_in_block,
                TEST_WASM_EXECUTION_POINTS,
                max_points_per_block,
                constants.max_block_execution_points,
            );
        }

        let committees = build_committees(self.committees);
        let num_committees = u32::try_from(committees.len()).expect("WAAAY too many committees");

        let leader_strategy = RoundRobinLeaderStrategy::new();
        let (tx_epoch_events, _) = broadcast::channel(10);
        let epoch_manager = TestEpochManager::new(tx_epoch_events);
        epoch_manager.add_committees(committees).await;
        if let Some((dest, claim_key)) = self.claim_keys {
            epoch_manager.set_claim_keys(dest, claim_key).await;
        }
        let shutdown = Shutdown::new();
        let (channels, validators) = Self::build_validators(
            &leader_strategy,
            &epoch_manager,
            self.rocks_path,
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
            submitted_transactions: Vec::new(),

            _leader_strategy: leader_strategy,
            _epoch_manager: epoch_manager,
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
        return ShardGroup::new(Shard::first(), Shard::from(num_shards.as_u32()));
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

    ShardGroup::new(start + 1, end)
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
