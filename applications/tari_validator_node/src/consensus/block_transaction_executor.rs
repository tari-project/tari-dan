//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::sync::Arc;

use indexmap::IndexMap;
use log::info;
use tari_consensus::traits::{BlockTransactionExecutor, BlockTransactionExecutorError};
use tari_dan_app_utilities::transaction_executor::TransactionExecutor;
use tari_dan_common_types::Epoch;
use tari_dan_engine::state_store::{memory::MemoryStateStore, new_memory_store, AtomicDb, StateWriter};
use tari_dan_storage::{consensus_models::ExecutedTransaction, StateStore};
use tari_engine_types::{
    substate::{Substate, SubstateId},
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

    fn add_substates_to_memory_db(
        &self,
        inputs: &IndexMap<SubstateId, Substate>,
        out: &MemoryStateStore,
    ) -> Result<(), BlockTransactionExecutorError> {
        // TODO: pass the impl SubstateStore directly into the engine
        let mut access = out
            .write_access()
            .map_err(|e| BlockTransactionExecutorError::StateStoreError(e.to_string()))?;
        for (id, substate) in inputs {
            access
                .set_state(id, substate)
                .map_err(|e| BlockTransactionExecutorError::StateStoreError(e.to_string()))?;
        }
        access
            .commit()
            .map_err(|e| BlockTransactionExecutorError::StateStoreError(e.to_string()))?;

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
        resolved_inputs: &IndexMap<SubstateId, Substate>,
    ) -> Result<ExecutedTransaction, BlockTransactionExecutorError> {
        let id = *transaction.id();

        // if let Some(abort_reason) = transaction.abort_reason() {
        //     // TODO: Hacky - if a transaction uses DOWNed/non-existent inputs we error here. This changes the hard
        //     // error to a propose REJECT. So that we have involved shards, we use the inputs as resolved inputs and
        //     // assume v0 if version is not provided.
        //     let inputs = transaction
        //         .transaction()
        //         .all_inputs_iter()
        //         .map(|input| VersionedSubstateId::new(input.substate_id, input.version.unwrap_or(0)))
        //         .map(|id| VersionedSubstateIdLockIntent::new(id, SubstateLockFlag::Write))
        //         .collect();
        //     return Ok(ExecutedTransaction::new(
        //         transaction.into_transaction(),
        //         ExecuteResult {
        //             finalize: FinalizeResult {
        //                 transaction_hash: id.into_array().into(),
        //                 events: vec![],
        //                 logs: vec![],
        //                 execution_results: vec![],
        //                 result: TransactionResult::Reject(abort_reason.clone()),
        //                 fee_receipt: Default::default(),
        //             },
        //         },
        //         inputs,
        //         vec![],
        //         Duration::from_secs(0),
        //     ));
        // }
        info!(target: LOG_TARGET, "Transaction {} executing. Inputs: {:?}", id, resolved_inputs);

        // Create a memory db with all the input substates, needed for the transaction execution
        let state_db = new_memory_store();
        self.add_substates_to_memory_db(resolved_inputs, &state_db)?;

        let mut virtual_substates = VirtualSubstates::new();
        virtual_substates.insert(
            VirtualSubstateId::CurrentEpoch,
            VirtualSubstate::CurrentEpoch(current_epoch.as_u64()),
        );

        // Execute the transaction and get the result
        let exec_output = self
            .executor
            .execute(transaction, state_db, virtual_substates)
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
