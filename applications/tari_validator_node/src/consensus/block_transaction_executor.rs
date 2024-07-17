//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::sync::Arc;

use indexmap::IndexMap;
use log::info;
use tari_consensus::{
    hotstuff::substate_store::PendingSubstateStore,
    traits::{BlockTransactionExecutor, BlockTransactionExecutorError, ReadableSubstateStore},
};
use tari_dan_app_utilities::transaction_executor::TransactionExecutor;
use tari_dan_common_types::{optional::Optional, Epoch};
use tari_dan_engine::state_store::{memory::MemoryStateStore, new_memory_store, AtomicDb, StateWriter};
use tari_dan_storage::{
    consensus_models::{ExecutedTransaction, TransactionRecord},
    StateStore,
};
use tari_engine_types::{
    substate::{Substate, SubstateId},
    virtual_substate::VirtualSubstates,
};
use tari_transaction::{Transaction, VersionedSubstateId};

use crate::{transaction_validators::TransactionValidationError, validator::Validator};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::block_transaction_executor";

#[derive(Debug)]
pub struct TariDanBlockTransactionExecutor<TEpochManager, TExecutor, TValidator> {
    // TODO: we will need the epoch manager for virtual substates and other operations in the future
    #[allow(dead_code)]
    epoch_manager: TEpochManager,
    executor: TExecutor,
    validator: Arc<TValidator>,
}

impl<TEpochManager, TExecutor, TValidator> TariDanBlockTransactionExecutor<TEpochManager, TExecutor, TValidator> {
    pub fn new(epoch_manager: TEpochManager, executor: TExecutor, validator: TValidator) -> Self {
        Self {
            epoch_manager,
            executor,
            validator: Arc::new(validator),
        }
    }

    fn resolve_substates<TStateStore: StateStore>(
        &self,
        transaction: &Transaction,
        store: &PendingSubstateStore<TStateStore>,
    ) -> Result<IndexMap<VersionedSubstateId, Substate>, BlockTransactionExecutorError> {
        let mut resolved_substates = IndexMap::with_capacity(transaction.num_unique_inputs());

        for input in transaction.all_inputs_iter() {
            match input.version() {
                Some(version) => {
                    let id = VersionedSubstateId::new(input.substate_id, version);
                    let substate = store.get(&id)?;
                    info!(target: LOG_TARGET, "Resolved substate: {id}");
                    resolved_substates.insert(id, substate);
                },
                None => {
                    let (id, substate) = self.resolve_local_substate::<TStateStore>(input.substate_id, store)?;
                    info!(target: LOG_TARGET, "Resolved unversioned substate: {id}");
                    resolved_substates.insert(id, substate);
                },
            }
        }
        // TODO: we assume local only transactions, we need to implement multi-shard transactions.
        //       Suggest once we have pledges for foreign substates, we add them to a temporary pledge store and use
        //       that to resolve inputs.
        Ok(resolved_substates)
    }

    fn resolve_local_substate<TStateStore: StateStore>(
        &self,
        id: SubstateId,
        store: &PendingSubstateStore<TStateStore>,
    ) -> Result<(VersionedSubstateId, Substate), BlockTransactionExecutorError> {
        let substate = store.get_latest(&id).optional()?.ok_or_else(|| {
            BlockTransactionExecutorError::UnableToResolveSubstateId {
                substate_id: id.clone(),
            }
        })?;

        Ok((VersionedSubstateId::new(id, substate.version()), substate))
    }

    fn add_substates_to_memory_db(
        &self,
        inputs: &IndexMap<VersionedSubstateId, Substate>,
        out: &MemoryStateStore,
    ) -> Result<(), BlockTransactionExecutorError> {
        // TODO: pass the impl SubstateStore directly into the engine
        let mut access = out
            .write_access()
            .map_err(|e| BlockTransactionExecutorError::StateStoreError(e.to_string()))?;
        for (id, substate) in inputs {
            access
                .set_state(id.substate_id(), substate)
                .map_err(|e| BlockTransactionExecutorError::StateStoreError(e.to_string()))?;
        }
        access
            .commit()
            .map_err(|e| BlockTransactionExecutorError::StateStoreError(e.to_string()))?;

        Ok(())
    }
}

impl<TEpochManager, TExecutor, TStateStore, TValidator> BlockTransactionExecutor<TStateStore>
    for TariDanBlockTransactionExecutor<TEpochManager, TExecutor, TValidator>
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

    fn prepare(
        &self,
        transaction: Transaction,
        store: &TStateStore,
    ) -> Result<TransactionRecord, BlockTransactionExecutorError> {
        let t = store.with_read_tx(|tx| TransactionRecord::get(tx, transaction.id()))?;
        Ok(t)
    }

    fn execute(
        &self,
        transaction: Transaction,
        store: &PendingSubstateStore<TStateStore>,
    ) -> Result<ExecutedTransaction, BlockTransactionExecutorError> {
        let id: tari_transaction::TransactionId = *transaction.id();

        // Get the latest input substates
        let inputs = self.resolve_substates::<TStateStore>(&transaction, store)?;
        info!(target: LOG_TARGET, "Transaction {} executing. Inputs: {:?}", id, inputs);

        // Create a memory db with all the input substates, needed for the transaction execution
        let state_db = new_memory_store();
        self.add_substates_to_memory_db(&inputs, &state_db)?;

        // TODO: create the virtual substates for execution
        let virtual_substates = VirtualSubstates::new();

        // Execute the transaction and get the result
        let exec_output = self
            .executor
            .execute(transaction, state_db, virtual_substates)
            .map_err(|e| BlockTransactionExecutorError::ExecutionThreadFailure(e.to_string()))?;

        // Generate the resolved inputs to set the specific version and required lock flag, as we know it after
        // execution
        let resolved_inputs = exec_output.resolve_inputs(inputs);

        let executed = ExecutedTransaction::new(
            exec_output.transaction,
            exec_output.result,
            resolved_inputs,
            exec_output.outputs,
            exec_output.execution_time,
        );
        info!(target: LOG_TARGET, "Transaction {} executed. {}", id,executed.result().finalize.result);
        Ok(executed)
    }
}

impl<TEpochManager: Clone, TExecutor: Clone, TValidator> Clone
    for TariDanBlockTransactionExecutor<TEpochManager, TExecutor, TValidator>
{
    fn clone(&self) -> Self {
        Self {
            epoch_manager: self.epoch_manager.clone(),
            executor: self.executor.clone(),
            validator: self.validator.clone(),
        }
    }
}
pub struct ValidationContext {
    pub current_epoch: Epoch,
}
