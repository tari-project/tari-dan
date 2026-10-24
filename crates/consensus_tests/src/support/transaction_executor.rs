//    Copyright 2024 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::HashMap,
    iter,
    sync::{
        Arc,
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
};

use tari_consensus::traits::{
    BlockTransactionExecutor,
    BlockTransactionExecutorError,
    BlockTransactionValidator,
    UncommittedChainTemplates,
};
use tari_consensus_types::Decision;
use tari_engine::state_store::{StateWriter, memory::MemoryStateStore, new_memory_store};
use tari_engine_types::{
    commit_result::AbortReason,
    substate::{Substate, SubstateId},
};
use tari_ootle_common_types::{
    Epoch,
    LockIntent,
    SubstateLockType,
    SubstateRequirement,
    SubstateVersion,
    VersionedSubstateId,
    displayable::Displayable,
    services::template_provider::TemplateProvider,
};
use tari_ootle_storage::{
    StateStore,
    consensus_models::{LockedEpoch, TransactionExecution, VersionedSubstateIdLockIntent},
};
use tari_ootle_transaction::{Transaction, TransactionId};
use tari_template_lib_types::{TemplateAddress, TransactionReceiptAddress, constants::TARI_TOKEN};

use crate::support::{
    TestAddress,
    TestStore,
    create_execution_result_for_transaction,
    executions_store::TestExecutionSpecStore,
};

#[derive(Debug, Clone)]
pub struct TestBlockTransactionProcessor {
    address: TestAddress,
    store: TestExecutionSpecStore,
    /// The validator's state store, whose durably committed templates back the uncommitted ones.
    state_store: TestStore,
    history: Arc<Mutex<HashMap<TransactionId, TestHistoricalExecution>>>,
    execute_counts: Arc<Mutex<HashMap<TransactionId, usize>>>,
    /// Executions on a thread consensus speculates on, which are named `ootle-exec-<n>`.
    speculation_thread_executions: Arc<AtomicUsize>,
    template_misses: Arc<Mutex<HashMap<TransactionId, usize>>>,
}

impl TestBlockTransactionProcessor {
    pub fn new(address: TestAddress, store: TestExecutionSpecStore, state_store: TestStore) -> Self {
        Self {
            address,
            store,
            state_store,
            history: Default::default(),
            execute_counts: Default::default(),
            speculation_thread_executions: Default::default(),
            template_misses: Default::default(),
        }
    }

    pub fn store(&self) -> &TestExecutionSpecStore {
        &self.store
    }

    pub fn get_history(&self) -> HashMap<TransactionId, TestHistoricalExecution> {
        self.history.lock().unwrap().clone()
    }

    /// The number of times this validator has executed `transaction_id`.
    pub fn get_execute_count(&self, transaction_id: &TransactionId) -> usize {
        self.execute_counts
            .lock()
            .unwrap()
            .get(transaction_id)
            .copied()
            .unwrap_or(0)
    }

    /// The number of executions this validator ran on a thread it speculates on.
    pub fn get_speculation_thread_execution_count(&self) -> usize {
        self.speculation_thread_executions.load(Ordering::Relaxed)
    }

    /// The number of times this validator executed `transaction_id` without finding the template it calls.
    pub fn get_template_miss_count(&self, transaction_id: &TransactionId) -> usize {
        self.template_misses
            .lock()
            .unwrap()
            .get(transaction_id)
            .copied()
            .unwrap_or(0)
    }

    /// Whether `template` resolves as on a validator: from the chain's uncommitted templates, then from durably
    /// committed state.
    fn resolves(
        &self,
        templates: &UncommittedChainTemplates,
        template: &TemplateAddress,
    ) -> Result<bool, BlockTransactionExecutorError> {
        if templates.get(template).is_some() {
            return Ok(true);
        }
        self.state_store
            .has_template(template)
            .map_err(|e| BlockTransactionExecutorError::StateStoreError(e.to_string()))
    }

    fn add_substates_to_memory_db<'a, I: IntoIterator<Item = (&'a SubstateRequirement, &'a Substate)>>(
        inputs: I,
        out: &mut MemoryStateStore,
    ) -> Result<(), BlockTransactionExecutorError> {
        // TODO: pass the impl SubstateStore directly into the engine
        for (id, substate) in inputs {
            out.set_state(id.substate_id().clone(), substate.clone())
                .map_err(|e| BlockTransactionExecutorError::StateStoreError(e.to_string()))?;
        }

        Ok(())
    }
}

impl BlockTransactionValidator for TestBlockTransactionProcessor {
    type Error = BlockTransactionExecutorError;

    fn validate_full(&self, _current_epoch: Epoch, _transaction: &Transaction) -> Result<(), Self::Error> {
        Ok(())
    }

    fn validate_epoch(&self, _current_epoch: Epoch, _transaction: &Transaction) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl<TStateStore: StateStore> BlockTransactionExecutor<TStateStore> for TestBlockTransactionProcessor {
    #[allow(clippy::too_many_lines)]
    fn execute(
        &self,
        transaction: &Transaction,
        execution_epoch: LockedEpoch,
        _input_fanout: u32,
        resolved_inputs: &HashMap<SubstateRequirement, Substate>,
        templates: &UncommittedChainTemplates,
    ) -> Result<TransactionExecution, BlockTransactionExecutorError> {
        let id = transaction.calculate_id();

        log::info!(
            "[{}] Transaction {} executing. {} input(s)",
            self.address,
            id,
            resolved_inputs.len()
        );

        // Create a memory db with all the input substates, needed for the transaction execution
        let mut state_db = new_memory_store();
        Self::add_substates_to_memory_db(resolved_inputs, &mut state_db)?;

        let spec = self.store.get(&id).unwrap_or_else(|| {
            panic!(
                "[{}] Missing execution spec for transaction {}",
                self.address,
                transaction.calculate_id()
            )
        });

        let resolved_inputs = spec
            .input_locks
            .into_iter()
            // Implicitly add XTR as read lock for all transactions
            .chain(iter::once((SubstateId::from(TARI_TOKEN), SubstateLockType::Read)))
            .map(|(substate_id, lock_type)| {
                let (requirement, substate) = resolved_inputs.get_key_value(&substate_id).unwrap_or_else(|| {
                    panic!(
                        "[{}] Consensus did not provide input substate {} for transaction {}. In spec: {}. Provided: \
                         {}",
                        self.address,
                        substate_id,
                        id,
                        lock_type,
                        resolved_inputs.keys().collect::<Vec<_>>().display()
                    )
                });
                VersionedSubstateIdLockIntent::new(
                    VersionedSubstateId::new(substate_id, substate.version()),
                    lock_type,
                    requirement.version().is_some(),
                )
                .with_pledged_value(substate.substate_value())
            })
            .collect::<Vec<_>>();

        let resulting_outputs = spec
            .new_outputs
            .into_iter()
            .map(|substate_id| VersionedSubstateId::new(substate_id, SubstateVersion::ZERO))
            // Generate corresponding up substates to all consumed inputs
            .chain(
                resolved_inputs.iter()
                    .filter(|input| input.lock_type().is_write() && !input.substate_id().is_validator_fee_pool())
                    .map(|input| input.versioned_substate_id().to_next_version()),
            )
            .chain(iter::once(VersionedSubstateId::new(
                SubstateId::TransactionReceipt(TransactionReceiptAddress::from(transaction.calculate_id())),
                SubstateVersion::ZERO,
            )))
            .map(VersionedSubstateIdLockIntent::output)
            .collect::<Vec<_>>();

        let decision = match spec.calls_template {
            Some(template) if !self.resolves(templates, &template.as_template_address())? => {
                log::info!(
                    "[{}] Transaction {} aborts: template {} not found",
                    self.address,
                    id,
                    template
                );
                *self.template_misses.lock().unwrap().entry(id).or_default() += 1;
                Decision::Abort(AbortReason::ExecutionFailure)
            },
            _ => spec.decision,
        };

        let result = create_execution_result_for_transaction(
            transaction,
            decision,
            spec.fee,
            &resolved_inputs,
            &resulting_outputs,
            spec.validator_fee_withdrawals,
        );

        let resulting_outputs = result
            .finalize
            .result
            .any_accept()
            .map(|diff| {
                diff.up_iter()
                    .map(|(addr, substate)| {
                        VersionedSubstateIdLockIntent::output(VersionedSubstateId::new(
                            addr.clone(),
                            substate.version(),
                        ))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let executed = TransactionExecution::new(result, resolved_inputs, resulting_outputs);
        log::info!(
            "[{}] Transaction {} executed in {:.2?}. {}",
            self.address,
            id,
            executed.execution_time(),
            executed.result().finalize.result
        );
        *self.execute_counts.lock().unwrap().entry(id).or_default() += 1;
        if thread::current()
            .name()
            .is_some_and(|name| name.starts_with("ootle-exec-"))
        {
            self.speculation_thread_executions.fetch_add(1, Ordering::Relaxed);
        }
        self.history.lock().unwrap().insert(id, TestHistoricalExecution {
            execution: executed.clone(),
            execution_epoch,
        });
        Ok(executed)
    }
}

#[derive(Debug, Clone)]
pub struct TestHistoricalExecution {
    pub execution: TransactionExecution,
    pub execution_epoch: LockedEpoch,
}
