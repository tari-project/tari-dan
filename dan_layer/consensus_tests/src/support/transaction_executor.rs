//    Copyright 2024 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashMap, iter};

use tari_consensus::traits::{BlockTransactionExecutor, BlockTransactionExecutorError};
use tari_dan_common_types::{Epoch, LockIntent, SubstateRequirement, VersionedSubstateId};
use tari_dan_engine::state_store::{memory::MemoryStateStore, new_memory_store, StateWriter};
use tari_dan_storage::{
    consensus_models::{ExecutedTransaction, VersionedSubstateIdLockIntent},
    StateStore,
};
use tari_engine_types::{
    substate::{Substate, SubstateId},
    transaction_receipt::TransactionReceiptAddress,
    virtual_substate::{VirtualSubstate, VirtualSubstateId, VirtualSubstates},
};
use tari_transaction::Transaction;

use crate::support::{create_execution_result_for_transaction, executions_store::TestExecutionSpecStore};

#[derive(Debug, Clone)]
pub struct TestBlockTransactionProcessor {
    store: TestExecutionSpecStore,
}

impl TestBlockTransactionProcessor {
    pub fn new(store: TestExecutionSpecStore) -> Self {
        Self { store }
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

impl<TStateStore: StateStore> BlockTransactionExecutor<TStateStore> for TestBlockTransactionProcessor {
    fn validate(
        &self,
        _tx: &TStateStore::ReadTransaction<'_>,
        _current_epoch: Epoch,
        _transaction: &Transaction,
    ) -> Result<(), BlockTransactionExecutorError> {
        Ok(())
    }

    fn execute(
        &self,
        transaction: Transaction,
        current_epoch: Epoch,
        resolved_inputs: &HashMap<SubstateRequirement, Substate>,
    ) -> Result<ExecutedTransaction, BlockTransactionExecutorError> {
        let id = *transaction.id();

        log::info!("Transaction {} executing. {} input(s)", id, resolved_inputs.len());

        // Create a memory db with all the input substates, needed for the transaction execution
        let mut state_db = new_memory_store();
        Self::add_substates_to_memory_db(resolved_inputs, &mut state_db)?;

        let mut virtual_substates = VirtualSubstates::new();
        virtual_substates.insert(
            VirtualSubstateId::CurrentEpoch,
            VirtualSubstate::CurrentEpoch(current_epoch.as_u64()),
        );

        let spec = self
            .store
            .get(transaction.id())
            .unwrap_or_else(|| panic!("Missing execution spec for transaction {}", transaction.id()));

        let resolved_inputs = spec
            .inputs
            .into_iter()
            .map(|spec| {
                let substate = resolved_inputs.get(spec.substate_requirement()).unwrap_or_else(|| {
                    panic!(
                        "Missing input substate for transaction {} with requirement {}",
                        id,
                        spec.substate_requirement()
                    )
                });
                VersionedSubstateIdLockIntent::new(
                    VersionedSubstateId::new(spec.substate_id().clone(), substate.version()),
                    spec.lock_type(),
                    spec.requested_version().is_some(),
                )
            })
            .collect::<Vec<_>>();

        let resulting_outputs = spec
            .new_outputs
            .into_iter()
            .map(|substate_id| VersionedSubstateId::new(substate_id, 0))
            // Generate corresponding up substates to all consumed inputs
            .chain(
                resolved_inputs.iter().filter(|input| input.lock_type().is_write())
                    .map(|input| input.versioned_substate_id().to_next_version()),
            )
            .chain(iter::once(VersionedSubstateId::new(
                SubstateId::TransactionReceipt(TransactionReceiptAddress::from(*transaction.id())),
                0,
            )))
            .map(VersionedSubstateIdLockIntent::output)
            .collect::<Vec<_>>();

        let exec_output = create_execution_result_for_transaction(
            transaction,
            spec.decision,
            spec.fee,
            &resolved_inputs,
            &resulting_outputs,
        );

        let executed = ExecutedTransaction::new(exec_output.transaction, exec_output.result, resolved_inputs);
        log::info!("Transaction {} executed. {}", id, executed.result().finalize.result);
        Ok(executed)
    }
}
