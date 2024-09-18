//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashMap, sync::Arc};

use log::info;
use tari_consensus::traits::{BlockTransactionExecutor, BlockTransactionExecutorError};
use tari_dan_app_utilities::transaction_executor::TransactionExecutor;
use tari_dan_common_types::{Epoch, SubstateRequirement};
use tari_dan_engine::state_store::{memory::MemoryStateStore, new_memory_store, StateWriter};
use tari_dan_storage::{consensus_models::ExecutedTransaction, StateStore};
use tari_engine_types::{
    substate::Substate,
    virtual_substate::{VirtualSubstate, VirtualSubstateId, VirtualSubstates},
};
use tari_transaction::Transaction;

use crate::{transaction_validators::TransactionValidationError, validator::Validator};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::block_transaction_executor";

#[derive(Debug)]
pub struct TariDanBlockTransactionExecutor<TExecutor, TValidator> {
    executor: TExecutor,
    validator: Arc<TValidator>,
}

impl<TExecutor, TValidator> TariDanBlockTransactionExecutor<TExecutor, TValidator>
where TExecutor: TransactionExecutor
{
    pub fn new(executor: TExecutor, validator: TValidator) -> Self {
        Self {
            executor,
            validator: Arc::new(validator),
        }
    }

    fn add_substates_to_memory_db<'a, I: IntoIterator<Item = (&'a SubstateRequirement, &'a Substate)>>(
        inputs: I,
        out: &mut MemoryStateStore,
    ) -> Result<(), BlockTransactionExecutorError> {
        // TODO: pass the SubstateStore directly into the engine
        for (id, substate) in inputs {
            out.set_state(id.substate_id().clone(), substate.clone())
                .map_err(|e| BlockTransactionExecutorError::StateStoreError(e.to_string()))?;
        }

        Ok(())
    }
}

impl<TExecutor, TStateStore, TValidator> BlockTransactionExecutor<TStateStore>
    for TariDanBlockTransactionExecutor<TExecutor, TValidator>
where
    TStateStore: StateStore,
    TExecutor: TransactionExecutor,
    for<'a> TValidator: Validator<Transaction, Context = ValidationContext, Error = TransactionValidationError>,
{
    fn validate(
        &self,
        _tx: &TStateStore::ReadTransaction<'_>,
        current_epoch: Epoch,
        transaction: &Transaction,
    ) -> Result<(), BlockTransactionExecutorError> {
        self.validator
            .validate(&ValidationContext { current_epoch }, transaction)
            // TODO: see if we can avoid the err as string
            .map_err(|e| BlockTransactionExecutorError::TransactionValidationError(e.to_string()))
    }

    fn execute(
        &self,
        transaction: Transaction,
        current_epoch: Epoch,
        resolved_inputs: &HashMap<SubstateRequirement, Substate>,
    ) -> Result<ExecutedTransaction, BlockTransactionExecutorError> {
        let id = *transaction.id();

        info!(target: LOG_TARGET, "Transaction {} executing. {} input(s)", id, resolved_inputs.len());

        // Create a memory db with all the input substates, needed for the transaction execution
        let mut state_db = new_memory_store();
        Self::add_substates_to_memory_db(resolved_inputs, &mut state_db)?;

        let mut virtual_substates = VirtualSubstates::new();
        virtual_substates.insert(
            VirtualSubstateId::CurrentEpoch,
            VirtualSubstate::CurrentEpoch(current_epoch.as_u64()),
        );

        // Execute the transaction and get the result
        let exec_output = self
            .executor
            .execute(transaction, state_db.into_read_only(), virtual_substates)
            .map_err(|e| BlockTransactionExecutorError::ExecutionThreadFailure(e.to_string()))?;

        // Generate the resolved inputs to set the specific version and required lock flag, as we know it after
        // execution
        let resolved_inputs = exec_output.resolve_inputs(resolved_inputs);

        let executed = ExecutedTransaction::new(exec_output.transaction, exec_output.result, resolved_inputs);
        info!(target: LOG_TARGET, "Transaction {} executed. {}", id,executed.result().finalize.result);
        Ok(executed)
    }
}

impl<TExecutor: Clone, TValidator> Clone for TariDanBlockTransactionExecutor<TExecutor, TValidator> {
    fn clone(&self) -> Self {
        Self {
            executor: self.executor.clone(),
            validator: self.validator.clone(),
        }
    }
}
pub struct ValidationContext {
    pub current_epoch: Epoch,
}
